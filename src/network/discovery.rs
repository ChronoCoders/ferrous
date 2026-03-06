use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::consensus::params::Network;
use crate::network::addrman::AddressManager;
use crate::network::manager::PeerManager;
use crate::network::protocol::{AddrMessage, NetworkAddr};

pub struct PeerDiscovery {
    addr_manager: Arc<Mutex<AddressManager>>,
    peer_manager: Arc<PeerManager>,
    target_outbound: usize,
}

impl PeerDiscovery {
    pub fn new(
        addr_manager: Arc<Mutex<AddressManager>>,
        peer_manager: Arc<PeerManager>,
        target_outbound: usize,
    ) -> Self {
        Self {
            addr_manager,
            peer_manager,
            target_outbound,
        }
    }

    pub fn start(&self) {
        let addr_manager = self.addr_manager.clone();
        let peer_manager = self.peer_manager.clone();
        let target = self.target_outbound;

        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(30));

                // Check current outbound count
                let current = peer_manager.outbound_peer_count();

                if current < target {
                    // Try to connect to new peers
                    let needed = target - current;
                    let addrs = addr_manager.lock().unwrap().get_random_addresses(needed);

                    for addr in addrs {
                        if peer_manager.connect_to_peer(addr).is_err() {
                            addr_manager.lock().unwrap().mark_failed(&addr);
                        } else {
                            addr_manager.lock().unwrap().mark_tried(&addr);
                        }
                    }
                }
            }
        });
    }

    pub fn handle_addr(&self, addrs: &AddrMessage) -> Result<(), String> {
        let mut addr_manager = self.addr_manager.lock().unwrap();

        for net_addr in &addrs.addresses {
            // Convert NetworkAddr to SocketAddr
            if let Some(addr) = network_addr_to_socket_addr(net_addr) {
                // Add to address manager
                addr_manager.add_address(addr, net_addr.services, net_addr.timestamp);
            }
        }

        Ok(())
    }

    pub fn handle_getaddr(&self, peer_id: u64) -> Result<(), String> {
        let addr_manager = self.addr_manager.lock().unwrap();
        let addresses = addr_manager.get_all_addresses();
        drop(addr_manager);

        // Send up to 1000 addresses
        let to_send: Vec<_> = addresses.into_iter().take(1000).collect();

        if to_send.is_empty() {
            return Ok(());
        }

        let addr_msg = AddrMessage { addresses: to_send };
        self.peer_manager
            .send_to_peer(peer_id, &self.create_addr_msg(addr_msg))?;

        Ok(())
    }
}

fn network_addr_to_socket_addr(net_addr: &NetworkAddr) -> Option<SocketAddr> {
    // Basic conversion, ignoring IPv6 complexity for now (assuming mapped IPv4 or valid IPv6)
    // protocol::NetAddr uses [u8; 16] for IP
    // Rust IpAddr::from([u8; 16]) creates IPv6
    // If it's IPv4 mapped (::ffff:1.2.3.4), we should probably convert to IPv4?
    // std::net::Ipv6Addr has methods for this.

    let ip = std::net::Ipv6Addr::from(net_addr.ip);
    if let Some(ipv4) = ip.to_ipv4_mapped() {
        Some(SocketAddr::new(IpAddr::V4(ipv4), net_addr.port))
    } else {
        Some(SocketAddr::new(IpAddr::V6(ip), net_addr.port))
    }
}

pub fn get_seed_nodes(network: Network) -> Vec<SocketAddr> {
    match network {
        Network::Regtest => vec!["127.0.0.1:18444".parse().unwrap()],
        Network::Testnet => vec![
            // Future: Add testnet seed nodes
        ],
        Network::Mainnet => vec![
            // Future: Add mainnet seed nodes
        ],
    }
}

// Extension trait removed, helper method used instead
use crate::network::message::NetworkMessage;
use crate::primitives::serialize::Encode;

impl PeerDiscovery {
    // Helper to fix the unimplemented above without changing the trait
    fn create_addr_msg(&self, addr_msg: AddrMessage) -> NetworkMessage {
        let magic = self.peer_manager.magic();
        NetworkMessage::new(magic, "addr", addr_msg.encode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::protocol::NetworkAddr;

    #[test]
    fn test_network_addr_conversion() {
        let ip: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 127, 0, 0, 1]; // ::ffff:127.0.0.1
        let net_addr = NetworkAddr {
            timestamp: 0,
            services: 1,
            ip,
            port: 8333,
        };

        let socket_addr = network_addr_to_socket_addr(&net_addr).unwrap();
        assert_eq!(socket_addr.ip().to_string(), "127.0.0.1");
        assert_eq!(socket_addr.port(), 8333);
    }
}
