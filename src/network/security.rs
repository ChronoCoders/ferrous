use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::time::{Duration, Instant};

// Security parameters
pub const MIN_SUBNET_DIVERSITY: usize = 8; // Min different /16 subnets
pub const MAX_PEERS_PER_SUBNET: usize = 3; // Max peers from same /16
pub const ANCHOR_CONNECTIONS: usize = 2; // Always-on trusted peers
pub const FEELER_INTERVAL: Duration = Duration::from_secs(120); // 2 minutes
pub const MAX_NETGROUP_PERCENTAGE: f32 = 0.30; // Max 30% from one netgroup

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetGroup {
    Ipv4Subnet16(u16), // First 16 bits of IPv4
    Ipv6Subnet32(u32), // First 32 bits of IPv6
    Local,
}

impl NetGroup {
    pub fn from_ip(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(addr) => {
                let octets = addr.octets();
                if octets[0] == 127 || octets[0] == 10 {
                    NetGroup::Local
                } else {
                    let subnet = ((octets[0] as u16) << 8) | (octets[1] as u16);
                    NetGroup::Ipv4Subnet16(subnet)
                }
            }
            IpAddr::V6(addr) => {
                let segments = addr.segments();
                if segments[0] == 0xfe80 || segments[0] == 0 {
                    NetGroup::Local
                } else {
                    let subnet = ((segments[0] as u32) << 16) | (segments[1] as u32);
                    NetGroup::Ipv6Subnet32(subnet)
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct NetworkSecurity {
    // Subnet tracking
    peers_by_netgroup: HashMap<NetGroup, Vec<u64>>,
    netgroups_by_peer: HashMap<u64, NetGroup>,

    // Anchor peers (trusted, persistent)
    anchor_peers: HashSet<u64>,

    // Feeler connections (test new addresses)
    last_feeler: Instant,
    feeler_peers: HashSet<u64>,

    // Eclipse detection
    connection_history: Vec<(Instant, IpAddr)>,
}

impl NetworkSecurity {
    pub fn new() -> Self {
        Self {
            peers_by_netgroup: HashMap::new(),
            netgroups_by_peer: HashMap::new(),
            anchor_peers: HashSet::new(),
            last_feeler: Instant::now(),
            feeler_peers: HashSet::new(),
            connection_history: Vec::new(),
        }
    }

    /// Check if connection maintains network diversity
    pub fn can_accept_for_diversity(&self, ip: IpAddr, total_peers: usize) -> bool {
        if total_peers == 0 {
            return true; // Always accept first peer
        }

        let netgroup = NetGroup::from_ip(ip);

        // Check subnet limit
        if let Some(peers) = self.peers_by_netgroup.get(&netgroup) {
            if peers.len() >= MAX_PEERS_PER_SUBNET {
                return false;
            }
        }

        // Check netgroup percentage
        let netgroup_count = self
            .peers_by_netgroup
            .get(&netgroup)
            .map(|v| v.len())
            .unwrap_or(0);
        let percentage = (netgroup_count as f32 + 1.0) / (total_peers as f32 + 1.0);

        if percentage > MAX_NETGROUP_PERCENTAGE {
            return false;
        }

        // Ensure minimum subnet diversity (only enforce after 10+ peers)
        if total_peers >= 10 {
            let unique_netgroups = self.peers_by_netgroup.len();
            if unique_netgroups < MIN_SUBNET_DIVERSITY
                && !self.peers_by_netgroup.contains_key(&netgroup)
            {
                // Allow if this adds a new netgroup
                return true;
            }
        }

        true
    }

    /// Record new peer connection
    pub fn record_peer(&mut self, peer_id: u64, ip: IpAddr) {
        let netgroup = NetGroup::from_ip(ip);

        self.peers_by_netgroup
            .entry(netgroup)
            .or_default()
            .push(peer_id);

        self.netgroups_by_peer.insert(peer_id, netgroup);

        // Track connection history for Eclipse detection
        self.connection_history.push((Instant::now(), ip));

        // Keep last 100 connections
        if self.connection_history.len() > 100 {
            self.connection_history.remove(0);
        }
    }

    /// Record peer disconnection
    pub fn remove_peer(&mut self, peer_id: u64) {
        if let Some(netgroup) = self.netgroups_by_peer.remove(&peer_id) {
            if let Some(peers) = self.peers_by_netgroup.get_mut(&netgroup) {
                peers.retain(|&id| id != peer_id);
                if peers.is_empty() {
                    self.peers_by_netgroup.remove(&netgroup);
                }
            }
        }

        self.anchor_peers.remove(&peer_id);
        self.feeler_peers.remove(&peer_id);
    }

    /// Mark peer as anchor (trusted, persistent)
    pub fn add_anchor(&mut self, peer_id: u64) {
        self.anchor_peers.insert(peer_id);
    }

    /// Check if peer is anchor
    pub fn is_anchor(&self, peer_id: u64) -> bool {
        self.anchor_peers.contains(&peer_id)
    }

    /// Get number of anchor connections
    pub fn anchor_count(&self) -> usize {
        self.anchor_peers.len()
    }

    /// Check if should create feeler connection
    pub fn should_create_feeler(&self) -> bool {
        self.last_feeler.elapsed() >= FEELER_INTERVAL && self.feeler_peers.len() < 2
    }

    /// Mark peer as feeler connection
    pub fn add_feeler(&mut self, peer_id: u64) {
        self.feeler_peers.insert(peer_id);
        self.last_feeler = Instant::now();
    }

    /// Check if peer is feeler
    pub fn is_feeler(&self, peer_id: u64) -> bool {
        self.feeler_peers.contains(&peer_id)
    }

    /// Get network diversity statistics
    pub fn get_diversity_stats(&self) -> (usize, usize, f32) {
        let unique_netgroups = self.peers_by_netgroup.len();
        let total_peers: usize = self.peers_by_netgroup.values().map(|v| v.len()).sum();

        let max_netgroup_size = self
            .peers_by_netgroup
            .values()
            .map(|v| v.len())
            .max()
            .unwrap_or(0);

        let max_percentage = if total_peers > 0 {
            max_netgroup_size as f32 / total_peers as f32
        } else {
            0.0
        };

        (unique_netgroups, total_peers, max_percentage)
    }

    /// Detect potential Eclipse attack
    pub fn detect_eclipse_attempt(&self) -> bool {
        // Check if many connections from same netgroup in short time
        // This check does NOT require 20 connections history.
        // It checks recent_5min.
        let recent_5min: Vec<_> = self
            .connection_history
            .iter()
            .filter(|(time, _)| time.elapsed() < Duration::from_secs(300))
            .collect();

        if recent_5min.len() >= 10 {
            let netgroup_counts: HashMap<NetGroup, usize> =
                recent_5min.iter().fold(HashMap::new(), |mut acc, (_, ip)| {
                    *acc.entry(NetGroup::from_ip(*ip)).or_insert(0) += 1;
                    acc
                });

            // If >7 connections from one netgroup in 5 minutes
            if netgroup_counts.values().any(|&count| count > 7) {
                return true;
            }
        }

        if self.connection_history.len() < 20 {
            return false;
        }

        // Check last 20 connections
        let recent: Vec<_> = self.connection_history.iter().rev().take(20).collect();

        // Count unique netgroups in recent connections
        let unique_netgroups: HashSet<_> = recent
            .iter()
            .map(|(_, ip)| NetGroup::from_ip(*ip))
            .collect();

        // If >15 of last 20 connections are from <=2 netgroups, suspicious
        if unique_netgroups.len() <= 2 && recent.len() >= 15 {
            return true;
        }

        false
    }

    /// Get peer for eviction (least diverse netgroup)
    pub fn select_eviction_candidate(&self) -> Option<u64> {
        // Find largest netgroup
        let largest = self
            .peers_by_netgroup
            .iter()
            .max_by_key(|(_, peers)| peers.len())?;

        // Don't evict anchors
        largest
            .1
            .iter()
            .find(|&&peer_id| !self.anchor_peers.contains(&peer_id))
            .copied()
    }
}

impl Default for NetworkSecurity {
    fn default() -> Self {
        Self::new()
    }
}
