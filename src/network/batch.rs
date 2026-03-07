use crate::network::message::NetworkMessage;
use crate::network::protocol::{
    AddrMessage, GetDataMessage, InvMessage, InvVector, MessagePayload, NetworkAddr,
};
use crate::primitives::serialize::Encode;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

// Batching parameters
pub const BATCH_INTERVAL: Duration = Duration::from_millis(100);
pub const MAX_INV_BATCH: usize = 1000;
pub const MAX_GETDATA_BATCH: usize = 1000;
pub const MAX_ADDR_BATCH: usize = 1000;

#[derive(Debug)]
pub struct MessageBatcher {
    // Pending inventory items per peer
    pending_inv: HashMap<u64, Vec<InvVector>>,
    pending_getdata: HashMap<u64, Vec<InvVector>>,
    pending_addr: HashMap<u64, Vec<NetworkAddr>>,

    // Time when the first item was added to the current batch for a peer
    batch_start_time: HashMap<u64, Instant>,

    // Magic for message creation
    magic: [u8; 4],
}

impl MessageBatcher {
    pub fn new(magic: [u8; 4]) -> Self {
        Self {
            pending_inv: HashMap::new(),
            pending_getdata: HashMap::new(),
            pending_addr: HashMap::new(),
            batch_start_time: HashMap::new(),
            magic,
        }
    }

    fn ensure_timer_started(&mut self, peer_id: u64) {
        self.batch_start_time
            .entry(peer_id)
            .or_insert_with(Instant::now);
    }

    /// Add inventory item to batch
    pub fn add_inv(&mut self, peer_id: u64, item: InvVector) {
        self.ensure_timer_started(peer_id);
        self.pending_inv.entry(peer_id).or_default().push(item);
    }

    /// Add getdata item to batch
    pub fn add_getdata(&mut self, peer_id: u64, item: InvVector) {
        self.ensure_timer_started(peer_id);
        self.pending_getdata.entry(peer_id).or_default().push(item);
    }

    /// Add addr to batch
    pub fn add_addr(&mut self, peer_id: u64, addr: NetworkAddr) {
        self.ensure_timer_started(peer_id);
        self.pending_addr.entry(peer_id).or_default().push(addr);
    }

    /// Check if peer needs flushing
    pub fn should_flush(&self, peer_id: u64) -> bool {
        // Check time-based flush
        if let Some(start) = self.batch_start_time.get(&peer_id) {
            if start.elapsed() >= BATCH_INTERVAL {
                return true;
            }
        }

        // Check size-based flush
        if let Some(inv) = self.pending_inv.get(&peer_id) {
            if inv.len() >= MAX_INV_BATCH {
                return true;
            }
        }

        if let Some(getdata) = self.pending_getdata.get(&peer_id) {
            if getdata.len() >= MAX_GETDATA_BATCH {
                return true;
            }
        }

        if let Some(addr) = self.pending_addr.get(&peer_id) {
            if addr.len() >= MAX_ADDR_BATCH {
                return true;
            }
        }

        false
    }

    /// Flush batched messages for peer
    pub fn flush(&mut self, peer_id: u64) -> Vec<NetworkMessage> {
        let mut messages = Vec::new();

        // Flush INV
        if let Some(items) = self.pending_inv.remove(&peer_id) {
            if !items.is_empty() {
                let payload = MessagePayload::Inv(InvMessage { inventory: items });
                let encoded = Encode::encode(&payload);
                // command string for Inv is "inv"
                let msg = NetworkMessage::new(self.magic, "inv", encoded);
                messages.push(msg);
            }
        }

        // Flush GetData
        if let Some(items) = self.pending_getdata.remove(&peer_id) {
            if !items.is_empty() {
                let payload = MessagePayload::GetData(GetDataMessage { inventory: items });
                let encoded = Encode::encode(&payload);
                let msg = NetworkMessage::new(self.magic, "getdata", encoded);
                messages.push(msg);
            }
        }

        // Flush Addr
        if let Some(addrs) = self.pending_addr.remove(&peer_id) {
            if !addrs.is_empty() {
                let payload = MessagePayload::Addr(AddrMessage { addresses: addrs });
                let encoded = Encode::encode(&payload);
                let msg = NetworkMessage::new(self.magic, "addr", encoded);
                messages.push(msg);
            }
        }

        // Reset timer
        self.batch_start_time.remove(&peer_id);
        messages
    }

    /// Flush all peers that need flushing
    pub fn flush_all_needed(&mut self) -> HashMap<u64, Vec<NetworkMessage>> {
        let mut result = HashMap::new();

        let peers: Vec<u64> = self
            .pending_inv
            .keys()
            .chain(self.pending_getdata.keys())
            .chain(self.pending_addr.keys())
            .copied()
            .collect();

        // Deduplicate peers
        let mut unique_peers = peers;
        unique_peers.sort_unstable();
        unique_peers.dedup();

        for peer_id in unique_peers {
            if self.should_flush(peer_id) {
                let messages = self.flush(peer_id);
                if !messages.is_empty() {
                    result.insert(peer_id, messages);
                }
            }
        }

        result
    }

    /// Clear peer state on disconnect
    pub fn clear_peer(&mut self, peer_id: u64) {
        self.pending_inv.remove(&peer_id);
        self.pending_getdata.remove(&peer_id);
        self.pending_addr.remove(&peer_id);
        self.batch_start_time.remove(&peer_id);
    }
}

/// Broadcast cache to avoid duplicate sends
#[derive(Debug)]
pub struct BroadcastCache {
    // Track what we've sent to each peer
    // Using VecDeque for FIFO eviction
    sent_to_peer: HashMap<u64, VecDeque<[u8; 32]>>,
    max_cache_size: usize,
}

impl BroadcastCache {
    pub fn new(max_cache_size: usize) -> Self {
        Self {
            sent_to_peer: HashMap::new(),
            max_cache_size,
        }
    }

    /// Check if already sent to peer
    pub fn already_sent(&self, peer_id: u64, hash: &[u8; 32]) -> bool {
        self.sent_to_peer
            .get(&peer_id)
            .map(|cache| cache.contains(hash))
            .unwrap_or(false)
    }

    /// Mark as sent to peer
    pub fn mark_sent(&mut self, peer_id: u64, hash: [u8; 32]) {
        let cache = self.sent_to_peer.entry(peer_id).or_default();

        if !cache.contains(&hash) {
            cache.push_back(hash);

            // Keep cache bounded
            if cache.len() > self.max_cache_size {
                cache.pop_front();
            }
        }
    }

    /// Clear peer cache on disconnect
    pub fn clear_peer(&mut self, peer_id: u64) {
        self.sent_to_peer.remove(&peer_id);
    }
}
