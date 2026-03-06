use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

// Connection limits
pub const MAX_TOTAL_CONNECTIONS: usize = 125;
pub const MAX_INBOUND_CONNECTIONS: usize = 100;
pub const MAX_OUTBOUND_CONNECTIONS: usize = 25;
pub const MAX_CONNECTIONS_PER_IP: usize = 3;
pub const MAX_FEELER_CONNECTIONS: usize = 5;

// Rate limiting
pub const CONNECTION_ATTEMPT_WINDOW: Duration = Duration::from_secs(60);
pub const MAX_ATTEMPTS_PER_WINDOW: usize = 10;
pub const FAILED_ATTEMPT_COOLDOWN: Duration = Duration::from_secs(300); // 5 min

// Resource limits
pub const MAX_MEMORY_PER_PEER: usize = 10 * 1024 * 1024; // 10 MB
pub const MAX_TOTAL_MEMORY: usize = 500 * 1024 * 1024; // 500 MB

#[derive(Debug, Clone)]
struct ConnectionAttempt {
    timestamp: Instant,
    #[allow(dead_code)] // Will be used for analytics later
    successful: bool,
}

#[derive(Debug)]
pub struct DosProtection {
    // Connection tracking
    connections_by_ip: HashMap<IpAddr, usize>,
    inbound_count: usize,
    outbound_count: usize,

    // Rate limiting
    attempts_by_ip: HashMap<IpAddr, Vec<ConnectionAttempt>>,
    failed_attempts: HashMap<IpAddr, Instant>,

    // Resource tracking
    memory_usage_by_peer: HashMap<u64, usize>,
    total_memory_usage: usize,
}

impl DosProtection {
    pub fn new() -> Self {
        Self {
            connections_by_ip: HashMap::new(),
            inbound_count: 0,
            outbound_count: 0,
            attempts_by_ip: HashMap::new(),
            failed_attempts: HashMap::new(),
            memory_usage_by_peer: HashMap::new(),
            total_memory_usage: 0,
        }
    }

    /// Check if new inbound connection should be accepted
    pub fn can_accept_inbound(&mut self, ip: IpAddr) -> bool {
        // Check total limits
        if self.inbound_count >= MAX_INBOUND_CONNECTIONS {
            return false;
        }

        // Check per-IP limit
        let count = self.connections_by_ip.get(&ip).unwrap_or(&0);
        if *count >= MAX_CONNECTIONS_PER_IP {
            return false;
        }

        // Check if IP is in cooldown (recent failed attempts)
        if let Some(cooldown_until) = self.failed_attempts.get(&ip) {
            if cooldown_until.elapsed() < FAILED_ATTEMPT_COOLDOWN {
                return false;
            }
        }

        // Check rate limiting
        self.cleanup_old_attempts();
        let attempts = self.attempts_by_ip.entry(ip).or_default();
        let recent_attempts = attempts
            .iter()
            .filter(|a| a.timestamp.elapsed() < CONNECTION_ATTEMPT_WINDOW)
            .count();

        if recent_attempts >= MAX_ATTEMPTS_PER_WINDOW {
            return false;
        }

        true
    }

    /// Check if new outbound connection should be attempted
    pub fn can_connect_outbound(&self, ip: IpAddr) -> bool {
        // Check total limits
        if self.outbound_count >= MAX_OUTBOUND_CONNECTIONS {
            return false;
        }

        // Check per-IP limit
        let count = self.connections_by_ip.get(&ip).unwrap_or(&0);
        if *count >= MAX_CONNECTIONS_PER_IP {
            return false;
        }

        // Check if IP is in cooldown
        if let Some(cooldown_until) = self.failed_attempts.get(&ip) {
            if cooldown_until.elapsed() < FAILED_ATTEMPT_COOLDOWN {
                return false;
            }
        }

        true
    }

    /// Record successful connection
    pub fn record_connection(&mut self, peer_id: u64, ip: IpAddr, inbound: bool) {
        // Update counts
        *self.connections_by_ip.entry(ip).or_insert(0) += 1;
        if inbound {
            self.inbound_count += 1;
        } else {
            self.outbound_count += 1;
        }

        // Record attempt
        self.attempts_by_ip
            .entry(ip)
            .or_default()
            .push(ConnectionAttempt {
                timestamp: Instant::now(),
                successful: true,
            });

        // Clear failed attempt cooldown
        self.failed_attempts.remove(&ip);

        // Initialize memory tracking
        self.memory_usage_by_peer.insert(peer_id, 0);
    }

    /// Record failed connection attempt
    pub fn record_failed_attempt(&mut self, ip: IpAddr) {
        self.attempts_by_ip
            .entry(ip)
            .or_default()
            .push(ConnectionAttempt {
                timestamp: Instant::now(),
                successful: false,
            });

        self.failed_attempts.insert(ip, Instant::now());
    }

    /// Record disconnection
    pub fn record_disconnection(&mut self, peer_id: u64, ip: IpAddr, inbound: bool) {
        // Update counts
        if let Some(count) = self.connections_by_ip.get_mut(&ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.connections_by_ip.remove(&ip);
            }
        }

        if inbound {
            self.inbound_count = self.inbound_count.saturating_sub(1);
        } else {
            self.outbound_count = self.outbound_count.saturating_sub(1);
        }

        // Clean up memory tracking
        if let Some(usage) = self.memory_usage_by_peer.remove(&peer_id) {
            self.total_memory_usage = self.total_memory_usage.saturating_sub(usage);
        }
    }

    /// Check if can allocate memory for peer
    pub fn can_allocate(&self, peer_id: u64, bytes: usize) -> bool {
        let current = self.memory_usage_by_peer.get(&peer_id).unwrap_or(&0);

        // Check per-peer limit
        if current + bytes > MAX_MEMORY_PER_PEER {
            return false;
        }

        // Check total limit
        if self.total_memory_usage + bytes > MAX_TOTAL_MEMORY {
            return false;
        }

        true
    }

    /// Record memory allocation
    pub fn allocate(&mut self, peer_id: u64, bytes: usize) {
        *self.memory_usage_by_peer.entry(peer_id).or_insert(0) += bytes;
        self.total_memory_usage += bytes;
    }

    /// Get current connection counts
    pub fn get_connection_counts(&self) -> (usize, usize, usize) {
        (
            self.inbound_count + self.outbound_count,
            self.inbound_count,
            self.outbound_count,
        )
    }

    /// Cleanup old connection attempts
    fn cleanup_old_attempts(&mut self) {
        let now = Instant::now();
        if now
            < self
                .attempts_by_ip
                .values()
                .flatten()
                .next()
                .map(|a| a.timestamp)
                .unwrap_or(now)
                + CONNECTION_ATTEMPT_WINDOW
        {
            // Optimization: only check if enough time passed or on every check?
            // Since we call this on inbound check, it's fine.
        }

        let cutoff = now
            .checked_sub(CONNECTION_ATTEMPT_WINDOW * 2)
            .unwrap_or(now);

        self.attempts_by_ip.retain(|_, attempts| {
            attempts.retain(|a| a.timestamp > cutoff);
            !attempts.is_empty()
        });
    }
}

impl Default for DosProtection {
    fn default() -> Self {
        Self::new()
    }
}
