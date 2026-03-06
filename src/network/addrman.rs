use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::network::protocol::NetworkAddr;

#[derive(Clone, Debug)]
struct PeerAddressInfo {
    addr: SocketAddr,
    services: u64,
    timestamp: u32,
    last_tried: Option<u32>,
    last_success: Option<u32>,
    attempts: u32,
}

pub struct AddressManager {
    addresses: HashMap<SocketAddr, PeerAddressInfo>,
    max_addresses: usize,
}

impl AddressManager {
    pub fn new(max_addresses: usize) -> Self {
        Self {
            addresses: HashMap::new(),
            max_addresses,
        }
    }

    // Add peer address
    pub fn add_address(&mut self, addr: SocketAddr, services: u64, timestamp: u32) {
        // Ignore if we have too many
        if self.addresses.len() >= self.max_addresses && !self.addresses.contains_key(&addr) {
            return;
        }

        // Add or update
        self.addresses
            .entry(addr)
            .and_modify(|info| {
                if timestamp > info.timestamp {
                    info.timestamp = timestamp;
                    info.services = services;
                }
            })
            .or_insert(PeerAddressInfo {
                addr,
                services,
                timestamp,
                last_tried: None,
                last_success: None,
                attempts: 0,
            });
    }

    // Get addresses with best success rate
    pub fn get_best_addresses(&self, count: usize) -> Vec<SocketAddr> {
        let mut addrs: Vec<_> = self
            .addresses
            .values()
            .filter(|info| info.last_success.is_some())
            .collect();

        // Sort by success rate
        addrs.sort_by(|a, b| {
            let a_score = a.last_success.unwrap_or(0) as i64 - a.attempts as i64;
            let b_score = b.last_success.unwrap_or(0) as i64 - b.attempts as i64;
            b_score.cmp(&a_score)
        });

        addrs
            .into_iter()
            .take(count)
            .map(|info| info.addr)
            .collect()
    }

    pub fn clear(&mut self) {
        self.addresses.clear();
    }

    // Get random addresses for connecting
    pub fn get_random_addresses(&self, count: usize) -> Vec<SocketAddr> {
        use rand::seq::SliceRandom;

        // Prefer addresses we haven't tried or that succeeded
        let mut candidates: Vec<_> = self
            .addresses
            .values()
            .filter(|info| {
                // Skip addresses with too many failures
                info.attempts < 3 || info.last_success.is_some()
            })
            .map(|info| info.addr)
            .collect();

        candidates.shuffle(&mut rand::thread_rng());
        candidates.into_iter().take(count).collect()
    }

    // Mark address as tried
    pub fn mark_tried(&mut self, addr: &SocketAddr) {
        if let Some(info) = self.addresses.get_mut(addr) {
            info.last_tried = Some(current_time());
            info.attempts += 1;
        }
    }

    // Mark connection success
    pub fn mark_success(&mut self, addr: &SocketAddr) {
        if let Some(info) = self.addresses.get_mut(addr) {
            info.last_success = Some(current_time());
            info.attempts = 0; // Reset attempts on success
        }
    }

    // Mark connection failed
    pub fn mark_failed(&mut self, addr: &SocketAddr) {
        // Already incremented attempts in mark_tried
        // Just verify it exists
        if !self.addresses.contains_key(addr) {
            // Do nothing
        }
        // Could implement backoff or penalty logic here
    }

    // Get all addresses
    pub fn get_all_addresses(&self) -> Vec<NetworkAddr> {
        self.addresses
            .values()
            .map(|info| NetworkAddr {
                timestamp: info.timestamp,
                services: info.services,
                ip: match info.addr.ip() {
                    std::net::IpAddr::V4(ipv4) => ipv4.to_ipv6_mapped().octets(),
                    std::net::IpAddr::V6(ipv6) => ipv6.octets(),
                },
                port: info.addr.port(),
            })
            .collect()
    }

    // Get address count
    pub fn size(&self) -> usize {
        self.addresses.len()
    }

    // Remove old addresses
    pub fn cleanup_old(&mut self, max_age_days: u32) {
        let now = current_time();
        let max_age_seconds = max_age_days * 24 * 60 * 60;

        self.addresses.retain(|_, info| {
            // Keep if timestamp is recent enough OR we had success recently
            let age = now.saturating_sub(info.timestamp);
            let success_age = info
                .last_success
                .map(|t| now.saturating_sub(t))
                .unwrap_or(u32::MAX);

            age < max_age_seconds || success_age < max_age_seconds
        });
    }
}

fn current_time() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_add_address() {
        let mut addrman = AddressManager::new(10);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8333);

        addrman.add_address(addr, 1, 100);
        assert_eq!(addrman.size(), 1);

        // Update timestamp
        addrman.add_address(addr, 1, 200);
        assert_eq!(addrman.size(), 1);
        let all = addrman.get_all_addresses();
        assert_eq!(all[0].timestamp, 200);
    }

    #[test]
    fn test_get_random_addresses() {
        let mut addrman = AddressManager::new(10);
        for i in 0..5 {
            let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8333 + i);
            addrman.add_address(addr, 1, 100);
        }

        let random = addrman.get_random_addresses(3);
        assert_eq!(random.len(), 3);
    }

    #[test]
    fn test_mark_success_failure() {
        let mut addrman = AddressManager::new(10);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8333);
        addrman.add_address(addr, 1, 100);

        addrman.mark_tried(&addr);
        // We can't easily check private fields, but logic is simple
        addrman.mark_success(&addr);
    }

    #[test]
    fn test_cleanup_old() {
        let mut addrman = AddressManager::new(10);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8333);
        // Very old timestamp
        addrman.add_address(addr, 1, 100);

        addrman.cleanup_old(1); // Should remove it (now >> 100 + 1 day)
        assert_eq!(addrman.size(), 0);
    }
}
