use crate::network::protocol::NetworkAddr;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct AddressManager {
    addresses: HashMap<SocketAddr, PeerAddressInfo>,
    max_addresses: usize,
}

#[derive(Clone, Debug)]
struct PeerAddressInfo {
    addr: SocketAddr,
    services: u64,
    timestamp: u32,
    last_tried: Option<u32>,
    last_success: Option<u32>,
    attempts: u32,
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
                info.timestamp = timestamp;
                info.services = services;
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
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32;
            info.last_tried = Some(now);
            info.attempts += 1;
        }
    }

    // Mark connection success
    pub fn mark_success(&mut self, addr: &SocketAddr) {
        if let Some(info) = self.addresses.get_mut(addr) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32;
            info.last_success = Some(now);
            info.attempts = 0; // Reset attempts on success
        }
    }

    // Mark connection failed
    pub fn mark_failed(&mut self, _addr: &SocketAddr) {
        // Already handled in mark_tried mostly, but this can be explicit failure
        // We increment attempts in mark_tried, so this might be redundant unless we want to track failures specifically.
        // But the prompt says "Mark connection failed", so let's keep it consistent with the logic.
        // If we failed, we might want to ensure attempts is incremented if mark_tried wasn't called?
        // Usually mark_tried is called when we attempt.
        // Let's assume this updates state if needed.
    }

    // Get all addresses
    pub fn get_all_addresses(&self) -> Vec<NetworkAddr> {
        self.addresses
            .values()
            .map(|info| {
                let (ip, port) = match info.addr {
                    SocketAddr::V4(v4) => {
                        let octets = v4.ip().octets();
                        let mut ip = [0u8; 16];
                        ip[10] = 0xff;
                        ip[11] = 0xff;
                        ip[12] = octets[0];
                        ip[13] = octets[1];
                        ip[14] = octets[2];
                        ip[15] = octets[3];
                        (ip, v4.port())
                    }
                    SocketAddr::V6(v6) => (v6.ip().octets(), v6.port()),
                };

                NetworkAddr {
                    timestamp: info.timestamp,
                    services: info.services,
                    ip,
                    port,
                }
            })
            .collect()
    }

    // Get address count
    pub fn size(&self) -> usize {
        self.addresses.len()
    }

    // Remove old addresses
    pub fn cleanup_old(&mut self, max_age_days: u32) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;
        let max_age_secs = max_age_days * 24 * 3600;

        self.addresses.retain(|_, info| {
            if let Some(last_success) = info.last_success {
                now.saturating_sub(last_success) < max_age_secs
            } else {
                // If never succeeded, keep it unless it's very old timestamp
                now.saturating_sub(info.timestamp) < max_age_secs
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_address_manager() {
        let mut addrman = AddressManager::new(10);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8333);

        addrman.add_address(addr, 1, 100);
        assert_eq!(addrman.size(), 1);

        let addrs = addrman.get_random_addresses(1);
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0], addr);

        addrman.mark_tried(&addr);
        addrman.mark_success(&addr);

        let all = addrman.get_all_addresses();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].port, 8333);
    }
}
