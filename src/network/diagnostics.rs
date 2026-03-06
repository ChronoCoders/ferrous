use crate::network::manager::PeerManager;
use crate::network::peer::PeerState;
use std::sync::Arc;
use std::time::Duration;

pub struct NetworkDiagnostics {
    peer_manager: Arc<PeerManager>,
}

impl NetworkDiagnostics {
    pub fn new(peer_manager: Arc<PeerManager>) -> Self {
        Self { peer_manager }
    }

    // Get detailed peer information
    pub fn get_peer_info(&self) -> Vec<PeerDiagnosticInfo> {
        let peers = self.peer_manager.get_peers();
        let peers_guard = peers.lock().unwrap();

        peers_guard
            .values()
            .map(|p| PeerDiagnosticInfo {
                peer_id: p.id,
                address: p.addr.to_string(),
                inbound: p.inbound,
                connected_duration: p.connection_duration(),
                last_message: p.time_since_last_recv(),
                version: p.version,
                services: p.services,
                start_height: p.start_height,
                ban_score: p.get_ban_score(),
                bytes_sent: p.get_bytes_sent(),
                bytes_received: p.get_bytes_received(),
                send_rate: p.get_send_rate(),
                recv_rate: p.get_recv_rate(),
                latency: None,    // Need to add latency tracking to Peer first
                messages_sent: 0, // Need to track message counts per peer
                messages_received: 0,
            })
            .collect()
    }

    // Get connection summary
    pub fn get_connection_summary(&self) -> ConnectionSummary {
        let peers = self.peer_manager.get_peers();
        let peers_guard = peers.lock().unwrap();

        let total_peers = peers_guard.len();
        let mut inbound_peers = 0;
        let mut outbound_peers = 0;
        let mut active_peers = 0;
        let mut total_bandwidth_in = 0.0;
        let mut total_bandwidth_out = 0.0;

        for peer in peers_guard.values() {
            if peer.inbound {
                inbound_peers += 1;
            } else {
                outbound_peers += 1;
            }

            if peer.state == PeerState::Active {
                active_peers += 1;
            }

            total_bandwidth_in += peer.get_recv_rate();
            total_bandwidth_out += peer.get_send_rate();
        }

        ConnectionSummary {
            total_peers,
            inbound_peers,
            outbound_peers,
            active_peers,
            avg_latency: None, // Calculate if latency available
            total_bandwidth_in,
            total_bandwidth_out,
        }
    }

    // Get network health score (0-100)
    pub fn get_health_score(&self) -> u8 {
        let summary = self.get_connection_summary();
        let mut score = 100u8;

        // Penalty for too few peers
        if summary.total_peers < 3 {
            score = score.saturating_sub(30);
        } else if summary.total_peers < 8 {
            score = score.saturating_sub(10);
        }

        // Penalty for no outbound connections
        if summary.outbound_peers == 0 {
            score = score.saturating_sub(40);
        }

        // Penalty for low bandwidth (if we have peers but no data)
        if summary.active_peers > 0 && summary.total_bandwidth_in < 1.0 {
            // Less than 1 byte/s average
            score = score.saturating_sub(15);
        }

        score
    }
}

#[derive(Clone, Debug)]
pub struct PeerDiagnosticInfo {
    pub peer_id: u64,
    pub address: String,
    pub inbound: bool,
    pub connected_duration: Duration,
    pub last_message: Duration, // Time since last message
    pub version: Option<u32>,
    pub services: u64,
    pub start_height: u32,
    pub ban_score: u32,

    // Performance metrics
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub send_rate: f64, // bytes/sec
    pub recv_rate: f64, // bytes/sec
    pub latency: Option<Duration>,

    // Message counts
    pub messages_sent: u64,
    pub messages_received: u64,
}

#[derive(Clone, Debug)]
pub struct ConnectionSummary {
    pub total_peers: usize,
    pub inbound_peers: usize,
    pub outbound_peers: usize,
    pub active_peers: usize,
    pub avg_latency: Option<Duration>,
    pub total_bandwidth_in: f64,  // bytes/sec
    pub total_bandwidth_out: f64, // bytes/sec
}
