use crate::consensus::params::Network;
use crate::network::addrman::AddressManager;
use crate::network::manager::PeerManager;
use crate::network::message::{NetworkMessage, CMD_ADDR};
use crate::network::protocol::{AddrMessage, NetworkAddr};
use crate::primitives::serialize::Encode;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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

    // Start discovery loop (background thread)
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
                        if let Err(_e) = peer_manager.connect_to_peer(addr) {
                            addr_manager.lock().unwrap().mark_failed(&addr);
                        } else {
                            addr_manager.lock().unwrap().mark_tried(&addr);
                        }
                    }
                }
            }
        });
    }

    // Request addresses from peer
    pub fn request_addresses(&self, peer_id: u64) -> Result<(), String> {
        let magic = self.peer_manager.magic();
        let msg = NetworkMessage::new(magic, "getaddr", Vec::new());
        self.peer_manager.send_to_peer(peer_id, &msg)
    }

    // Handle received addresses
    pub fn handle_addr(&self, addrs: &AddrMessage) -> Result<(), String> {
        let mut addr_manager = self.addr_manager.lock().unwrap();

        for net_addr in &addrs.addresses {
            // Convert NetworkAddr to SocketAddr
            let addr = network_addr_to_socket_addr(net_addr);

            // Add to address manager
            addr_manager.add_address(addr, net_addr.services, net_addr.timestamp);
        }

        Ok(())
    }

    // Handle getaddr request
    pub fn handle_getaddr(&self, peer_id: u64) -> Result<(), String> {
        let addr_manager = self.addr_manager.lock().unwrap();
        let addresses = addr_manager.get_all_addresses();
        drop(addr_manager);

        // Send up to 1000 addresses
        let to_send: Vec<_> = addresses.into_iter().take(1000).collect();

        let addr_msg = AddrMessage { addresses: to_send };

        let magic = self.peer_manager.magic();
        let msg = NetworkMessage::new(magic, CMD_ADDR, addr_msg.encode());

        self.peer_manager.send_to_peer(peer_id, &msg)?;

        Ok(())
    }
}

fn network_addr_to_socket_addr(net_addr: &NetworkAddr) -> SocketAddr {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    // Check if IPv4-mapped IPv6
    if net_addr.ip[0..10].iter().all(|&x| x == 0)
        && net_addr.ip[10] == 0xff
        && net_addr.ip[11] == 0xff
    {
        let ip = Ipv4Addr::new(
            net_addr.ip[12],
            net_addr.ip[13],
            net_addr.ip[14],
            net_addr.ip[15],
        );
        SocketAddr::new(IpAddr::V4(ip), net_addr.port)
    } else {
        let ip = Ipv6Addr::from(net_addr.ip);
        SocketAddr::new(IpAddr::V6(ip), net_addr.port)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::message::REGTEST_MAGIC;

    #[test]
    fn test_peer_discovery_logic() {
        let addr_manager = Arc::new(Mutex::new(AddressManager::new(100)));
        let peer_manager = Arc::new(PeerManager::new(REGTEST_MAGIC, 70015, 0, 0, 10));

        let discovery = PeerDiscovery::new(addr_manager.clone(), peer_manager.clone(), 8);

        // Handle empty addr message
        let msg = AddrMessage { addresses: vec![] };
        assert!(discovery.handle_addr(&msg).is_ok());

        // Handle getaddr
        // Needs a peer to send to, so this might fail if send_to_peer checks for peer existence
        // PeerManager returns Err("Peer not found") if peer doesn't exist.
        assert!(discovery.handle_getaddr(999).is_err());
    }
}
