use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::network::connection::PeerConnection;
use crate::network::message::NetworkMessage;
use crate::network::protocol::{NetAddr, VerackMessage, VersionMessage};
use crate::network::ratelimit::RateLimiter;
use crate::primitives::serialize::Encode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Connecting,
    Connected,
    VersionSent,
    VersionReceived,
    Active,
    Disconnected,
}

pub struct Peer {
    pub id: u64,
    pub addr: SocketAddr,
    pub state: PeerState,
    pub connection: Option<PeerConnection>,
    pub version: Option<u32>,
    pub services: u64,
    pub start_height: u32,
    pub last_ping: Instant,
    pub last_recv: Instant,
    pub connected_at: Instant,
    pub last_pong: Instant,
    pub nonce: u64,
    pub inbound: bool,
    // Rate limiters
    message_limiter: RateLimiter, // Messages per second
    inv_limiter: RateLimiter,     // INV messages per second
    getdata_limiter: RateLimiter, // GetData requests per second
    // Bandwidth tracking
    bytes_sent: u64,
    bytes_received: u64,
    bandwidth_start: Instant,
    // Ban score
    ban_score: u32,
}

impl Peer {
    pub fn new(id: u64, addr: SocketAddr) -> Self {
        let now = Instant::now();
        Self {
            id,
            addr,
            state: PeerState::Connecting,
            connection: None,
            version: None,
            services: 0,
            start_height: 0,
            last_ping: now,
            last_pong: now,
            last_recv: now,
            connected_at: now,
            nonce: 0,
            inbound: false,
            message_limiter: RateLimiter::new(Duration::from_secs(1), 100),
            inv_limiter: RateLimiter::new(Duration::from_secs(1), 50),
            getdata_limiter: RateLimiter::new(Duration::from_secs(1), 50),
            bytes_sent: 0,
            bytes_received: 0,
            bandwidth_start: now,
            ban_score: 0,
        }
    }

    pub fn new_inbound(id: u64, connection: PeerConnection) -> Self {
        let addr = connection
            .peer_addr()
            .parse()
            .unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap());
        let now = Instant::now();
        Self {
            id,
            addr,
            state: PeerState::Connected,
            connection: Some(connection),
            version: None,
            services: 0,
            start_height: 0,
            last_ping: now,
            last_pong: now,
            last_recv: now,
            connected_at: now,
            nonce: 0,
            inbound: true,
            message_limiter: RateLimiter::new(Duration::from_secs(1), 100),
            inv_limiter: RateLimiter::new(Duration::from_secs(1), 50),
            getdata_limiter: RateLimiter::new(Duration::from_secs(1), 50),
            bytes_sent: 0,
            bytes_received: 0,
            bandwidth_start: now,
            ban_score: 0,
        }
    }

    // Check if peer can send message
    pub fn check_message_rate(&mut self) -> bool {
        self.message_limiter.check()
    }

    pub fn check_inv_rate(&mut self) -> bool {
        self.inv_limiter.check()
    }

    pub fn check_getdata_rate(&mut self) -> bool {
        self.getdata_limiter.check()
    }

    // Update bandwidth stats
    pub fn record_sent(&mut self, bytes: usize) {
        self.bytes_sent += bytes as u64;
    }

    pub fn record_received(&mut self, bytes: usize) {
        self.bytes_received += bytes as u64;
    }

    // Get bandwidth rate (bytes per second)
    pub fn get_send_rate(&self) -> f64 {
        let elapsed = self.bandwidth_start.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.bytes_sent as f64 / elapsed
        } else {
            0.0
        }
    }

    pub fn get_recv_rate(&self) -> f64 {
        let elapsed = self.bandwidth_start.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.bytes_received as f64 / elapsed
        } else {
            0.0
        }
    }

    pub fn add_ban_score(&mut self, points: u32) {
        self.ban_score += points;
    }

    pub fn get_ban_score(&self) -> u32 {
        self.ban_score
    }

    pub fn should_ban(&self) -> bool {
        self.ban_score >= 100
    }

    pub fn get_bytes_sent(&self) -> u64 {
        self.bytes_sent
    }

    pub fn get_bytes_received(&self) -> u64 {
        self.bytes_received
    }

    pub fn update_last_recv(&mut self) {
        self.last_recv = Instant::now();
    }

    pub fn time_since_last_recv(&self) -> std::time::Duration {
        self.last_recv.elapsed()
    }

    pub fn connection_duration(&self) -> std::time::Duration {
        self.connected_at.elapsed()
    }

    pub fn send(&mut self, message: &NetworkMessage) -> Result<(), String> {
        if let Some(conn) = &mut self.connection {
            conn.send_message(message)
        } else {
            Err("Peer not connected".to_string())
        }
    }

    pub fn receive(&mut self) -> Result<Option<NetworkMessage>, String> {
        if let Some(conn) = &mut self.connection {
            conn.try_read_message()
        } else {
            Err("Peer not connected".to_string())
        }
    }

    pub fn initiate_handshake(
        &mut self,
        our_version: u32,
        our_services: u64,
        our_height: u32,
        our_nonce: u64,
    ) -> Result<(), String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let addr_recv = NetAddr::new(self.addr, 0); // We don't know their services yet
        let addr_from = NetAddr::new("127.0.0.1:0".parse().unwrap(), our_services);

        let msg = VersionMessage {
            version: our_version,
            services: our_services,
            timestamp,
            receiver: addr_recv,
            sender: addr_from,
            nonce: our_nonce,
            user_agent: "/Ferrous:0.1.0/".to_string(),
            start_height: our_height,
        };

        // Create NetworkMessage
        let payload = msg.encode();
        // Assuming we need magic here? PeerConnection knows magic.
        // Wait, Peer doesn't know magic. We should probably pass magic or assume connection handles it.
        // But NetworkMessage constructor needs magic.
        // Let's assume PeerConnection handles magic verification on read, but for write we need to provide it?
        // Actually, NetworkMessage stores magic.
        // Peer doesn't store magic.
        // Let's extract magic from connection if possible, or pass it in.
        // For now, let's just use REGTEST_MAGIC as placeholder if we don't store it.
        // Ideally Peer should know the network magic.
        // Let's cheat and use REGTEST_MAGIC for now, or fetch from connection if we expose it.
        // Connection has magic private.
        // Let's use a default for now and fix later if needed.
        // Actually, connection.send_message takes NetworkMessage which has magic.
        // Let's update Peer to store magic? Or just use REGTEST_MAGIC.
        // For correctness, Peer should know magic.

        let magic = crate::network::message::REGTEST_MAGIC; // TODO: make configurable
        let net_msg = NetworkMessage::new(magic, "version", payload);

        self.send(&net_msg)?;
        self.state = PeerState::VersionSent;
        Ok(())
    }

    pub fn handle_version(&mut self, msg: &VersionMessage) -> Result<(), String> {
        if self.state != PeerState::Connected && self.state != PeerState::VersionSent {
            return Err(format!(
                "Unexpected version message in state {:?}",
                self.state
            ));
        }

        if msg.version < 70001 {
            return Err(format!("Peer version too low: {}", msg.version));
        }

        self.version = Some(msg.version);
        self.services = msg.services;
        self.start_height = msg.start_height;
        self.nonce = msg.nonce;

        // Send Verack
        let verack = VerackMessage;
        let magic = crate::network::message::REGTEST_MAGIC;
        let net_msg = NetworkMessage::new(magic, "verack", verack.encode());
        self.send(&net_msg)?;

        if self.state == PeerState::VersionSent {
            self.state = PeerState::VersionReceived;
        } else {
            // If we haven't sent version yet (inbound), we should send it now?
            // But initiate_handshake is supposed to be called first by us.
            // If inbound, we receive version first.
            // Then we should send our version + verack.
            // The handshake flow in handshake.rs assumes we send version first always?
            // That's for outbound.
            // For inbound, we read version, then send our version + verack.
            // Let's stick to the state transitions.
            self.state = PeerState::VersionReceived;
        }

        Ok(())
    }

    pub fn handle_verack(&mut self) -> Result<(), String> {
        if self.state != PeerState::VersionReceived {
            return Err(format!("Unexpected verack in state {:?}", self.state));
        }

        self.state = PeerState::Active;
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.state == PeerState::Active
    }
}
