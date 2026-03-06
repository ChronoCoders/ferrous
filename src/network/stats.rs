use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct NetworkStats {
    stats: Arc<Mutex<NetworkStatsData>>,
}

struct NetworkStatsData {
    start_time: Instant,

    // Connection stats
    total_connections_accepted: u64,
    total_connections_initiated: u64,
    total_connections_failed: u64,
    current_connections: usize,

    // Message stats
    messages_sent: u64,
    messages_received: u64,
    bytes_sent: u64,
    bytes_received: u64,

    // Protocol stats
    blocks_received: u64,
    blocks_sent: u64,
    transactions_received: u64,
    transactions_sent: u64,

    // Error stats
    invalid_messages: u64,
    rate_limited: u64,
    banned_peers: u64,
}

impl Default for NetworkStats {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkStats {
    pub fn new() -> Self {
        Self {
            stats: Arc::new(Mutex::new(NetworkStatsData {
                start_time: Instant::now(),
                total_connections_accepted: 0,
                total_connections_initiated: 0,
                total_connections_failed: 0,
                current_connections: 0,
                messages_sent: 0,
                messages_received: 0,
                bytes_sent: 0,
                bytes_received: 0,
                blocks_received: 0,
                blocks_sent: 0,
                transactions_received: 0,
                transactions_sent: 0,
                invalid_messages: 0,
                rate_limited: 0,
                banned_peers: 0,
            })),
        }
    }

    pub fn record_connection_accepted(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.total_connections_accepted += 1;
        stats.current_connections += 1;
    }

    pub fn record_connection_initiated(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.total_connections_initiated += 1;
        stats.current_connections += 1;
    }

    pub fn record_connection_failed(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.total_connections_failed += 1;
    }

    pub fn record_connection_closed(&self) {
        let mut stats = self.stats.lock().unwrap();
        if stats.current_connections > 0 {
            stats.current_connections -= 1;
        }
    }

    pub fn record_message_sent(&self, bytes: usize) {
        let mut stats = self.stats.lock().unwrap();
        stats.messages_sent += 1;
        stats.bytes_sent += bytes as u64;
    }

    pub fn record_message_received(&self, bytes: usize) {
        let mut stats = self.stats.lock().unwrap();
        stats.messages_received += 1;
        stats.bytes_received += bytes as u64;
    }

    pub fn record_block_received(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.blocks_received += 1;
    }

    pub fn record_block_sent(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.blocks_sent += 1;
    }

    pub fn record_transaction_received(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.transactions_received += 1;
    }

    #[allow(dead_code)]
    pub fn record_transaction_sent(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.transactions_sent += 1;
    }

    pub fn record_invalid_message(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.invalid_messages += 1;
    }

    pub fn record_rate_limited(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.rate_limited += 1;
    }

    pub fn record_banned_peer(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.banned_peers += 1;
    }

    // Get current stats
    pub fn get_snapshot(&self) -> NetworkStatsSnapshot {
        let stats = self.stats.lock().unwrap();
        let uptime = stats.start_time.elapsed().as_secs();
        let uptime_f64 = uptime.max(1) as f64;

        NetworkStatsSnapshot {
            uptime_secs: uptime,
            total_connections_accepted: stats.total_connections_accepted,
            total_connections_initiated: stats.total_connections_initiated,
            total_connections_failed: stats.total_connections_failed,
            current_connections: stats.current_connections,
            messages_sent: stats.messages_sent,
            messages_received: stats.messages_received,
            bytes_sent: stats.bytes_sent,
            bytes_received: stats.bytes_received,
            avg_send_rate: stats.bytes_sent as f64 / uptime_f64,
            avg_recv_rate: stats.bytes_received as f64 / uptime_f64,
            blocks_received: stats.blocks_received,
            blocks_sent: stats.blocks_sent,
            transactions_received: stats.transactions_received,
            invalid_messages: stats.invalid_messages,
            rate_limited: stats.rate_limited,
            banned_peers: stats.banned_peers,
        }
    }

    pub fn log_periodic_summary(&self) {
        let snapshot = self.get_snapshot();

        println!("=== Network Statistics ===");
        println!("Uptime: {} seconds", snapshot.uptime_secs);
        println!("Connections: {} active", snapshot.current_connections);
        println!(
            "Messages: {} sent, {} received",
            snapshot.messages_sent, snapshot.messages_received
        );
        println!(
            "Bandwidth: {:.2} KB/s in, {:.2} KB/s out",
            snapshot.avg_recv_rate / 1024.0,
            snapshot.avg_send_rate / 1024.0
        );
        println!(
            "Blocks: {} received, {} sent",
            snapshot.blocks_received, snapshot.blocks_sent
        );
        println!(
            "Errors: {} invalid, {} rate limited, {} banned",
            snapshot.invalid_messages, snapshot.rate_limited, snapshot.banned_peers
        );
    }
}

#[derive(Clone, Debug)]
pub struct NetworkStatsSnapshot {
    pub uptime_secs: u64,
    pub total_connections_accepted: u64,
    pub total_connections_initiated: u64,
    pub total_connections_failed: u64,
    pub current_connections: usize,
    pub messages_sent: u64,
    pub messages_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub avg_send_rate: f64, // bytes/sec
    pub avg_recv_rate: f64, // bytes/sec
    pub blocks_received: u64,
    pub blocks_sent: u64,
    pub transactions_received: u64,
    pub invalid_messages: u64,
    pub rate_limited: u64,
    pub banned_peers: u64,
}
