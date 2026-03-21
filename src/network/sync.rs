use crate::consensus::block::{Block, U256};
use crate::consensus::chain::ChainState;
use crate::consensus::difficulty::validate_difficulty;
use crate::consensus::validation::validate_timestamp;
use crate::network::manager::PeerManager;
use crate::network::message::NetworkMessage;
use crate::network::protocol::{
    GetDataMessage, GetHeadersMessage, HeadersMessage, InvMessage, InvVector, INV_BLOCK,
};
use crate::primitives::serialize::Encode;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Number of blocks each peer may have in-flight simultaneously.
const DOWNLOAD_WINDOW: usize = 64;

/// Global cap on total in-flight block requests across all peers.
const MAX_INFLIGHT: usize = 512;

/// Re-request a block if no response arrives within this window.
const BLOCK_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum number of orphan blocks held in memory.
const ORPHAN_POOL_MAX: usize = 2048;

type PeerId = u64;
type LocatorEntry = ([u8; 32], u64);
type Locator = Vec<LocatorEntry>;

// ---------------------------------------------------------------------------
// BlockDownloadQueue — work-stealing parallel block fetcher
// ---------------------------------------------------------------------------

/// Tracks which block hashes still need to be downloaded, which are currently
/// in-flight (requested but not yet received), and how many in-flight requests
/// each peer currently carries.
///
/// Invariants:
/// - A hash appears in at most one of `pending` or `in_flight`.
/// - `peer_load[p]` equals the number of entries in `in_flight` whose peer is `p`.
/// - `in_flight.len() <= MAX_INFLIGHT` after every `drain_to_peers` call.
struct BlockDownloadQueue {
    /// Hashes not yet dispatched, ordered FIFO (oldest first = lowest height first).
    pending: VecDeque<[u8; 32]>,
    /// Hashes dispatched to a peer but not yet received: hash → (peer_id, sent_at).
    in_flight: HashMap<[u8; 32], (PeerId, Instant)>,
    /// Number of in-flight requests currently assigned to each peer.
    peer_load: HashMap<PeerId, usize>,
}

impl BlockDownloadQueue {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            in_flight: HashMap::new(),
            peer_load: HashMap::new(),
        }
    }

    /// Add hashes to the pending queue.  Hashes already in-flight are skipped
    /// (they will be re-queued automatically if they time out).
    fn enqueue_batch(&mut self, hashes: Vec<[u8; 32]>) {
        for hash in hashes {
            if !self.in_flight.contains_key(&hash) {
                self.pending.push_back(hash);
            }
        }
    }

    /// Push a single hash to the *front* of the pending queue (priority re-request).
    fn requeue_front(&mut self, hash: [u8; 32]) {
        // Remove from in_flight accounting first if it was there.
        if let Some((pid, _)) = self.in_flight.remove(&hash) {
            *self.peer_load.entry(pid).or_insert(0) = self
                .peer_load
                .get(&pid)
                .copied()
                .unwrap_or(0)
                .saturating_sub(1);
        }
        self.pending.push_front(hash);
    }

    /// Distribute pending hashes across `peers`, respecting per-peer window and
    /// global cap.  Returns a list of `(peer_id, hashes_to_request)` pairs.
    ///
    /// Peers are filled lightest-first so work spreads evenly.
    fn drain_to_peers(&mut self, peers: &[PeerId]) -> Vec<(PeerId, Vec<[u8; 32]>)> {
        if self.pending.is_empty() || peers.is_empty() {
            return vec![];
        }

        // Sort peers by ascending load so we fill the least-loaded first.
        let mut sorted: Vec<PeerId> = peers.to_vec();
        sorted.sort_by_key(|pid| self.peer_load.get(pid).copied().unwrap_or(0));

        let mut result: Vec<(PeerId, Vec<[u8; 32]>)> = Vec::new();

        'outer: for peer_id in sorted {
            let load = self.peer_load.get(&peer_id).copied().unwrap_or(0);
            let capacity = DOWNLOAD_WINDOW.saturating_sub(load);
            if capacity == 0 {
                continue;
            }
            let mut batch: Vec<[u8; 32]> = Vec::new();
            for _ in 0..capacity {
                if self.in_flight.len() >= MAX_INFLIGHT {
                    break 'outer;
                }
                match self.pending.pop_front() {
                    Some(hash) => {
                        self.in_flight.insert(hash, (peer_id, Instant::now()));
                        *self.peer_load.entry(peer_id).or_insert(0) += 1;
                        batch.push(hash);
                    }
                    None => break,
                }
            }
            if !batch.is_empty() {
                result.push((peer_id, batch));
            }
        }

        result
    }

    /// Mark a block as received.  Removes from in_flight and decrements peer_load.
    fn mark_received(&mut self, hash: &[u8; 32]) {
        if let Some((pid, _)) = self.in_flight.remove(hash) {
            *self.peer_load.entry(pid).or_insert(0) = self
                .peer_load
                .get(&pid)
                .copied()
                .unwrap_or(0)
                .saturating_sub(1);
        }
    }

    /// Called when a peer disconnects.  Moves all of its in-flight hashes back to
    /// the front of the pending queue so another peer can take over.
    fn on_peer_disconnected(&mut self, peer_id: PeerId) {
        let mut returned: Vec<[u8; 32]> = self
            .in_flight
            .iter()
            .filter(|(_, (pid, _))| *pid == peer_id)
            .map(|(hash, _)| *hash)
            .collect();

        for hash in &returned {
            self.in_flight.remove(hash);
        }
        self.peer_load.remove(&peer_id);

        // Return them to the front in reverse order so the lowest-height hash
        // ends up at the very front (original ordering was FIFO push_back).
        returned.sort_unstable();
        for hash in returned.into_iter().rev() {
            self.pending.push_front(hash);
        }
    }

    /// Move any in-flight hashes that have exceeded BLOCK_REQUEST_TIMEOUT back
    /// to the front of the pending queue for re-dispatch.
    fn recheck_timeouts(&mut self) {
        let mut timed_out: Vec<([u8; 32], PeerId)> = self
            .in_flight
            .iter()
            .filter(|(_, (_, sent_at))| sent_at.elapsed() > BLOCK_REQUEST_TIMEOUT)
            .map(|(hash, (pid, _))| (*hash, *pid))
            .collect();

        for (hash, pid) in &timed_out {
            self.in_flight.remove(hash);
            *self.peer_load.entry(*pid).or_insert(0) = self
                .peer_load
                .get(pid)
                .copied()
                .unwrap_or(0)
                .saturating_sub(1);
        }

        timed_out.sort_unstable_by_key(|(h, _)| *h);
        for (hash, _) in timed_out.into_iter().rev() {
            self.pending.push_front(hash);
        }
    }

    /// Reset all state (used when starting a fresh sync session).
    fn clear(&mut self) {
        self.pending.clear();
        self.in_flight.clear();
        self.peer_load.clear();
    }
}

// ---------------------------------------------------------------------------
// SyncState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum SyncState {
    Idle,
    DownloadingHeaders {
        peer_id: PeerId,
        best_header_height: u32,
        best_header_hash: [u8; 32],
        fork_start_height: Option<u32>,
    },
    DownloadingBlocks {
        /// All peers currently participating in the download.
        active_peers: Vec<PeerId>,
        /// Next block height we must apply before advancing.
        next_apply_height: u32,
        /// Highest block height in the peer's chain (sync target).
        total_height: u32,
    },
    Synced,
}

// ---------------------------------------------------------------------------
// SyncManager
// ---------------------------------------------------------------------------

pub struct SyncManager {
    chain: Arc<RwLock<ChainState>>,
    peer_manager: Arc<PeerManager>,
    sync_state: Arc<Mutex<SyncState>>,
    header_height_map: Arc<Mutex<HashMap<[u8; 32], u32>>>,
    last_locator: Arc<Mutex<Locator>>,
    /// Peer's header chain from the current sync session: height → hash.
    peer_header_map: Arc<Mutex<HashMap<u32, [u8; 32]>>>,
    /// Reverse of peer_header_map for O(1) block lookup: hash → height.
    peer_hash_to_height: Arc<Mutex<HashMap<[u8; 32], u32>>>,
    /// Blocks received out-of-order, waiting for their predecessor to be applied.
    block_buffer: Arc<Mutex<HashMap<u32, Block>>>,
    /// Work-stealing parallel download queue.
    download_queue: Arc<Mutex<BlockDownloadQueue>>,
    /// Orphan blocks whose parent is not yet known.
    orphan_pool: Arc<Mutex<HashMap<[u8; 32], Block>>>,
    /// Timestamp of the last recheck_stalled_window execution (for rate limiting).
    last_recheck: Mutex<Instant>,
}

impl SyncManager {
    pub fn new(chain: Arc<RwLock<ChainState>>, peer_manager: Arc<PeerManager>) -> Self {
        Self {
            chain,
            peer_manager,
            sync_state: Arc::new(Mutex::new(SyncState::Idle)),
            header_height_map: Arc::new(Mutex::new(HashMap::new())),
            last_locator: Arc::new(Mutex::new(Vec::new())),
            peer_header_map: Arc::new(Mutex::new(HashMap::new())),
            peer_hash_to_height: Arc::new(Mutex::new(HashMap::new())),
            block_buffer: Arc::new(Mutex::new(HashMap::new())),
            download_queue: Arc::new(Mutex::new(BlockDownloadQueue::new())),
            orphan_pool: Arc::new(Mutex::new(HashMap::new())),
            last_recheck: Mutex::new(Instant::now()),
        }
    }

    pub fn get_local_height(&self) -> u32 {
        let chain = self.chain.read().unwrap();
        chain
            .get_tip()
            .map(|t| t.map(|d| d.height).unwrap_or(0))
            .unwrap_or(0) as u32
    }

    pub fn is_syncing(&self) -> bool {
        let state = self.sync_state.lock().unwrap();
        matches!(
            *state,
            SyncState::DownloadingBlocks { .. } | SyncState::DownloadingHeaders { .. }
        )
    }

    // -----------------------------------------------------------------------
    // Block receive path
    // -----------------------------------------------------------------------

    /// Called by the relay for every received block.
    ///
    /// Returns `None`  → not a tracked sync block; relay should handle via normal add_block.
    /// Returns `Some(applied)` → sync handled this block (applied or buffered).
    ///   Each entry is `(block, height)` for a block successfully added to the chain.
    ///   An empty vec means the block was buffered and no apply happened yet.
    pub fn receive_block_for_sync(&self, block: Block) -> Option<Vec<(Block, u32)>> {
        let block_hash = block.header.hash();

        // Is this block part of the current sync session?
        let height = self
            .peer_hash_to_height
            .lock()
            .unwrap()
            .get(&block_hash)
            .copied()?;

        // Remove from in-flight (unconditional — keeps accounting correct for stale/duplicate).
        self.download_queue
            .lock()
            .unwrap()
            .mark_received(&block_hash);

        // Read the current download cursor.
        let (next_apply_height, active_peers, total_height) = {
            let state = self.sync_state.lock().unwrap();
            match &*state {
                SyncState::DownloadingBlocks {
                    active_peers,
                    next_apply_height,
                    total_height,
                } => (*next_apply_height, active_peers.clone(), *total_height),
                _ => {
                    // Arrived before we transitioned to DownloadingBlocks — buffer it.
                    self.block_buffer.lock().unwrap().insert(height, block);
                    return Some(vec![]);
                }
            }
        };

        if height < next_apply_height {
            // Stale/duplicate — already applied.
            return Some(vec![]);
        }

        if height > next_apply_height {
            // Out-of-order — buffer it.
            let buf_len = {
                let mut buf = self.block_buffer.lock().unwrap();
                buf.insert(height, block);
                buf.len()
            };
            // Buffer is growing; the block at next_apply_height may have been lost.
            // Re-queue it immediately so it gets re-requested on the next drain.
            if buf_len >= DOWNLOAD_WINDOW {
                if let Some(h) = self
                    .peer_header_map
                    .lock()
                    .unwrap()
                    .get(&next_apply_height)
                    .copied()
                {
                    self.download_queue.lock().unwrap().requeue_front(h);
                }
            }
            self.drain_to_peers_and_send(&active_peers);
            return Some(vec![]);
        }

        // height == next_apply_height.
        let expected_hash = self.peer_header_map.lock().unwrap().get(&height).copied();
        if expected_hash != Some(block_hash) {
            log::warn!(
                "SyncManager: hash mismatch at height {}: expected {} got {} — discarding",
                height,
                expected_hash
                    .map(hex::encode)
                    .unwrap_or_else(|| "none".into()),
                hex::encode(block_hash)
            );
            if let Some(h) = expected_hash {
                self.download_queue.lock().unwrap().requeue_front(h);
            }
            self.drain_to_peers_and_send(&active_peers);
            return Some(vec![]);
        }

        // Prev-hash guard (height > 0).
        if height > 0 {
            let expected_prev = self
                .peer_header_map
                .lock()
                .unwrap()
                .get(&(height - 1))
                .copied();
            if let Some(ep) = expected_prev {
                if block.header.prev_block_hash != ep {
                    log::warn!(
                        "SyncManager: prev_hash mismatch at height {} — discarding and re-requesting",
                        height
                    );
                    let expected_hash = self.peer_header_map.lock().unwrap().get(&height).copied();
                    if let Some(h) = expected_hash {
                        self.download_queue.lock().unwrap().requeue_front(h);
                    }
                    self.drain_to_peers_and_send(&active_peers);
                    return Some(vec![]);
                }
            }
        }

        // Apply the block and drain any consecutive buffered blocks.
        let mut applied: Vec<(Block, u32)> = Vec::new();
        let mut current_height = height;
        let mut current_block = block;

        loop {
            let result = {
                let mut chain = self.chain.write().unwrap();
                chain.add_block(current_block.clone())
            };

            match result {
                Ok(()) => {
                    let applied_hash = current_block.header.hash();
                    applied.push((current_block, current_height));
                    current_height += 1;

                    // Try any orphans whose parent is now in the chain.
                    let orphans = self.drain_orphans_for_parent(applied_hash);
                    for orphan in orphans {
                        let result = { self.chain.write().unwrap().add_block(orphan.clone()) };
                        if result.is_ok() {
                            log::debug!(
                                "SyncManager: applied orphan {} from pool",
                                hex::encode(orphan.header.hash())
                            );
                        }
                    }

                    if current_height > total_height {
                        let mut state = self.sync_state.lock().unwrap();
                        *state = SyncState::Synced;
                        log::info!("SyncManager: sync complete at height {}", total_height);
                        break;
                    }

                    {
                        let mut state = self.sync_state.lock().unwrap();
                        *state = SyncState::DownloadingBlocks {
                            active_peers: active_peers.clone(),
                            next_apply_height: current_height,
                            total_height,
                        };
                    }

                    self.drain_to_peers_and_send(&active_peers);

                    let next_block = self.block_buffer.lock().unwrap().remove(&current_height);
                    match next_block {
                        Some(b) => {
                            let exp = self
                                .peer_header_map
                                .lock()
                                .unwrap()
                                .get(&current_height)
                                .copied();
                            if exp != Some(b.header.hash()) {
                                log::warn!(
                                    "SyncManager: buffered block hash mismatch at height \
                                     {} — stopping drain",
                                    current_height
                                );
                                break;
                            }
                            current_block = b;
                        }
                        None => break,
                    }
                }
                Err(e) => {
                    log::warn!(
                        "SyncManager: add_block at height {} failed: {:?}, aborting ordered apply",
                        current_height,
                        e
                    );
                    break;
                }
            }
        }

        Some(applied)
    }

    // -----------------------------------------------------------------------
    // Stall detection
    // -----------------------------------------------------------------------

    /// Check for timed-out in-flight requests and re-dispatch them.
    /// Rate-limited to at most once every 10 seconds.
    pub fn recheck_stalled_window(&self) {
        const MIN_RECHECK_INTERVAL: Duration = Duration::from_secs(10);
        {
            let mut last = self.last_recheck.lock().unwrap();
            if last.elapsed() < MIN_RECHECK_INTERVAL {
                return;
            }
            *last = Instant::now();
        }

        let active_peers = {
            let state = self.sync_state.lock().unwrap();
            match &*state {
                SyncState::DownloadingBlocks { active_peers, .. } => active_peers.clone(),
                _ => return,
            }
        };

        // Move timed-out in-flight entries back to the pending queue.
        self.download_queue.lock().unwrap().recheck_timeouts();

        // Re-dispatch any pending work.
        self.drain_to_peers_and_send(&active_peers);
    }

    /// Spawn a background thread that calls `recheck_stalled_window` every 30 seconds.
    pub fn start_stall_checker(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("sync-stall-checker".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(30));
                match weak.upgrade() {
                    Some(mgr) => mgr.recheck_stalled_window(),
                    None => break,
                }
            })
            .expect("failed to spawn sync-stall-checker thread");
    }

    // -----------------------------------------------------------------------
    // Multi-peer dispatch
    // -----------------------------------------------------------------------

    /// Distribute pending download queue to the given peer list and send getdata.
    fn drain_to_peers_and_send(&self, peers: &[PeerId]) {
        let batches = self.download_queue.lock().unwrap().drain_to_peers(peers);
        let magic = self.peer_manager.magic();
        for (peer_id, hashes) in batches {
            if hashes.is_empty() {
                continue;
            }
            let getdata = GetDataMessage {
                inventory: hashes
                    .iter()
                    .map(|h| InvVector {
                        inv_type: INV_BLOCK,
                        hash: *h,
                    })
                    .collect(),
            };
            let msg = NetworkMessage::new(magic, "getdata", getdata.encode());
            let _ = self.peer_manager.send_to_peer(peer_id, &msg);
            log::debug!(
                "SyncManager: dispatched {} blocks to peer {}",
                hashes.len(),
                peer_id
            );
        }
    }

    /// Called by the manager when a peer disconnects during block download.
    /// Returns in-flight hashes for that peer to the pending queue and
    /// redistributes to remaining peers.
    pub fn on_peer_disconnected(&self, peer_id: PeerId) {
        self.download_queue
            .lock()
            .unwrap()
            .on_peer_disconnected(peer_id);

        let active_peers = {
            let mut state = self.sync_state.lock().unwrap();
            if let SyncState::DownloadingBlocks { active_peers, .. } = &mut *state {
                active_peers.retain(|&p| p != peer_id);
                if active_peers.is_empty() {
                    log::warn!(
                        "SyncManager: all download peers disconnected, waiting for reconnect"
                    );
                }
                active_peers.clone()
            } else {
                return;
            }
        };

        if !active_peers.is_empty() {
            self.drain_to_peers_and_send(&active_peers);
        }
    }

    // -----------------------------------------------------------------------
    // Orphan pool
    // -----------------------------------------------------------------------

    /// Store an orphan block (parent unknown). Evicts an arbitrary entry when full.
    pub fn store_orphan(&self, hash: [u8; 32], block: Block) {
        let mut pool = self.orphan_pool.lock().unwrap();
        if pool.len() >= ORPHAN_POOL_MAX {
            if let Some(key) = pool.keys().next().copied() {
                pool.remove(&key);
            }
        }
        pool.insert(hash, block);
    }

    /// Remove and return all orphans whose prev_block_hash matches `parent_hash`.
    fn drain_orphans_for_parent(&self, parent_hash: [u8; 32]) -> Vec<Block> {
        let mut pool = self.orphan_pool.lock().unwrap();
        let mut result = Vec::new();
        pool.retain(|_, block| {
            if block.header.prev_block_hash == parent_hash {
                result.push(block.clone());
                false
            } else {
                true
            }
        });
        result
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn find_first_missing(&self, total_height: u32) -> u32 {
        let peer_map = self.peer_header_map.lock().unwrap();
        let chain = self.chain.read().unwrap();

        let mut sorted: Vec<u32> = peer_map.keys().cloned().collect();
        sorted.sort_unstable();

        for h in sorted {
            if let Some(&hash) = peer_map.get(&h) {
                match chain.block_store.get_block(&hash) {
                    Ok(Some(_)) => {}
                    _ => return h,
                }
            }
        }

        total_height + 1
    }

    fn reset_sync_state(&self) {
        self.peer_header_map.lock().unwrap().clear();
        self.peer_hash_to_height.lock().unwrap().clear();
        self.block_buffer.lock().unwrap().clear();
        self.download_queue.lock().unwrap().clear();
        self.orphan_pool.lock().unwrap().clear();
    }

    fn compute_cumulative_work_for_tip(&self, tip_hash: [u8; 32]) -> Result<U256, String> {
        let chain = self.chain.read().unwrap();
        let mut cursor = tip_hash;
        let mut works: Vec<U256> = Vec::new();

        loop {
            if let Some(meta) = chain
                .block_store
                .get_block_meta(&cursor)
                .map_err(|e| e.to_string())?
            {
                let mut total = meta.cumulative_work;
                for w in works.iter().rev() {
                    total = total + *w;
                }
                return Ok(total);
            }

            let header = chain
                .block_store
                .get_header(&cursor)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Missing header {}", hex::encode(cursor)))?;

            works.push(header.work());

            if header.prev_block_hash == [0u8; 32] {
                let mut total = U256::from(0u64);
                for w in works.iter().rev() {
                    total = total + *w;
                }
                return Ok(total);
            }

            cursor = header.prev_block_hash;
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    // Legacy notification — kept for API compatibility.
    pub fn block_received(&self, _hash: [u8; 32]) {}

    #[allow(dead_code)]
    fn select_best_peer(&self) -> Option<u64> {
        let peers = self.peer_manager.get_connected_peers();
        let mut best_peer: Option<u64> = None;
        let mut best_height: u32 = 0;
        for peer_id in peers {
            let h = self
                .peer_manager
                .get_peer_start_height(peer_id)
                .unwrap_or(0);
            if best_peer.is_none() || h > best_height {
                best_peer = Some(peer_id);
                best_height = h;
            }
        }
        best_peer
    }

    pub fn start_sync(&self, peer_id: u64) -> Result<(), String> {
        log::debug!("SyncManager: Starting sync check with peer {}", peer_id);

        {
            let mut state = self.sync_state.lock().unwrap();
            match &mut *state {
                SyncState::DownloadingHeaders { .. } => {
                    log::debug!(
                        "SyncManager: Already downloading headers, skipping duplicate sync."
                    );
                    return Ok(());
                }
                SyncState::DownloadingBlocks { active_peers, .. } => {
                    // A new peer connected while we're already downloading blocks.
                    // Add them to the active set and immediately drain pending to them.
                    if !active_peers.contains(&peer_id) {
                        active_peers.push(peer_id);
                        log::info!(
                            "SyncManager: peer {} joined active download ({} peers now)",
                            peer_id,
                            active_peers.len()
                        );
                    }
                    drop(state);
                    self.drain_to_peers_and_send(&[peer_id]);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Not currently syncing — start fresh.
        self.reset_sync_state();

        let chain = self.chain.read().unwrap();
        let our_height = chain.get_height() as u32;

        let (best_header_height, best_header_hash) = match chain.state_store.load_best_header() {
            Ok(Some((h, hash))) => (h, hash),
            _ => {
                let tip = chain.get_tip().unwrap_or(None);
                if let Some(t) = tip {
                    (t.height as u32, t.block.header.hash())
                } else {
                    (0, [0u8; 32])
                }
            }
        };

        let (start_height, start_hash) = if best_header_height > our_height {
            (best_header_height, best_header_hash)
        } else {
            let tip = chain.get_tip().unwrap_or(None);
            if let Some(t) = tip {
                (t.height as u32, t.block.header.hash())
            } else {
                (0, [0u8; 32])
            }
        };

        let locator_with_heights = chain.get_block_locator_with_heights();
        let locator: Vec<[u8; 32]> = locator_with_heights.iter().map(|(h, _)| *h).collect();
        drop(chain);

        {
            let mut last = self.last_locator.lock().unwrap();
            *last = locator_with_heights;
        }

        let peer_height = self
            .peer_manager
            .get_peer_start_height(peer_id)
            .unwrap_or(0);
        log::debug!(
            "SyncManager: Local height: {}, Best Header: {}, Peer {} height: {}",
            our_height,
            start_height,
            peer_id,
            peer_height
        );

        {
            let mut state = self.sync_state.lock().unwrap();
            *state = SyncState::DownloadingHeaders {
                peer_id,
                best_header_height: start_height,
                best_header_hash: start_hash,
                fork_start_height: None,
            };
        }

        let getheaders = GetHeadersMessage {
            version: 70015,
            block_locator: locator,
            stop_hash: [0u8; 32],
        };

        let magic = self.peer_manager.magic();
        let msg = NetworkMessage::new(magic, "getheaders", getheaders.encode());
        self.peer_manager.send_to_peer(peer_id, &msg)?;

        let chain = self.chain.read().unwrap();
        let local_tip_hash = chain
            .get_tip()
            .unwrap_or(None)
            .map(|t| t.block.header.hash());
        drop(chain);
        if let Some(tip_hash) = local_tip_hash {
            let inv = InvMessage {
                inventory: vec![InvVector {
                    inv_type: INV_BLOCK,
                    hash: tip_hash,
                }],
            };
            let inv_net = NetworkMessage::new(magic, "inv", inv.encode());
            let _ = self.peer_manager.send_to_peer(peer_id, &inv_net);
        }

        Ok(())
    }

    pub fn request_headers_force(&self, peer_id: u64) -> Result<(), String> {
        self.start_sync(peer_id)
    }

    // -----------------------------------------------------------------------
    // Header processing
    // -----------------------------------------------------------------------

    pub fn handle_headers(&self, peer_id: u64, headers_msg: &HeadersMessage) -> Result<(), String> {
        log::debug!(
            "SyncManager: Received {} headers from peer {}",
            headers_msg.headers.len(),
            peer_id
        );

        if headers_msg.headers.is_empty() {
            let (best_height_opt, best_hash_opt, fork_start_height_opt, sync_peer_id) = {
                let state = self.sync_state.lock().unwrap();
                match *state {
                    SyncState::DownloadingHeaders {
                        best_header_height,
                        best_header_hash,
                        fork_start_height,
                        peer_id: spid,
                        ..
                    } => (
                        Some(best_header_height),
                        Some(best_header_hash),
                        fork_start_height,
                        spid,
                    ),
                    _ => (None, None, None, peer_id),
                }
            };

            let (best_height, best_hash) =
                if let (Some(h), Some(hash)) = (best_height_opt, best_hash_opt) {
                    (h, hash)
                } else {
                    let chain = self.chain.read().unwrap();
                    chain
                        .state_store
                        .load_best_header()
                        .unwrap_or(None)
                        .unwrap_or((0, [0u8; 32]))
                };

            let local_height = self.get_local_height();
            let peer_map_len = self.peer_header_map.lock().unwrap().len();
            if peer_map_len > 0 {
                let chain = self.chain.read().unwrap();
                let local_work = chain
                    .state_store
                    .get_tip()
                    .ok()
                    .flatten()
                    .map(|t| t.cumulative_work)
                    .unwrap_or(U256::from(0u64));
                drop(chain);

                let peer_work = if best_hash != [0u8; 32] {
                    self.compute_cumulative_work_for_tip(best_hash).ok()
                } else {
                    None
                };

                let first_missing = self.find_first_missing(best_height);
                if first_missing <= best_height {
                    let mut start = first_missing;
                    while start > 0 {
                        let parent_height = start - 1;
                        let hash_opt = self
                            .peer_header_map
                            .lock()
                            .unwrap()
                            .get(&parent_height)
                            .copied();
                        if let Some(hash) = hash_opt {
                            let chain = self.chain.read().unwrap();
                            match chain.block_store.get_block(&hash) {
                                Ok(Some(_)) => break,
                                _ => {
                                    start = parent_height;
                                }
                            }
                        } else {
                            let chain = self.chain.read().unwrap();
                            match chain.block_store.get_hash_by_height(parent_height as u64) {
                                Ok(Some(hash)) => {
                                    let have_block =
                                        chain.block_store.get_block(&hash).ok().flatten().is_some();
                                    drop(chain);
                                    self.peer_header_map
                                        .lock()
                                        .unwrap()
                                        .insert(parent_height, hash);
                                    self.peer_hash_to_height
                                        .lock()
                                        .unwrap()
                                        .insert(hash, parent_height);
                                    if have_block {
                                        break;
                                    } else {
                                        start = parent_height;
                                    }
                                }
                                _ => break,
                            }
                        }
                    }

                    let should_download = if local_height < best_height {
                        true
                    } else if let Some(pw) = peer_work {
                        pw > local_work || (pw == local_work && fork_start_height_opt.is_some())
                    } else {
                        fork_start_height_opt.is_some()
                    };

                    log::debug!(
                        "SyncManager: empty-headers decision: local_height={} best_height={} first_missing={} start={} peer_work={:?} local_work={:?} fork_start={:?} download={}",
                        local_height,
                        best_height,
                        first_missing,
                        start,
                        peer_work.map(|w| w.0),
                        local_work.0,
                        fork_start_height_opt,
                        should_download
                    );

                    if should_download && start <= best_height {
                        let hashes = self.collect_pending_hashes(start, best_height);
                        self.download_queue.lock().unwrap().enqueue_batch(hashes);

                        let mut state = self.sync_state.lock().unwrap();
                        *state = SyncState::DownloadingBlocks {
                            active_peers: vec![sync_peer_id],
                            next_apply_height: start,
                            total_height: best_height,
                        };
                        drop(state);

                        self.drain_to_peers_and_send(&[sync_peer_id]);
                        return Ok(());
                    }
                }
            }

            let mut state = self.sync_state.lock().unwrap();
            *state = SyncState::Synced;
            return Ok(());
        }

        let state = self.sync_state.lock().unwrap();

        let (mut current_best_height, mut current_best_hash, mut fork_start_height) = match *state {
            SyncState::DownloadingHeaders {
                best_header_height,
                best_header_hash,
                fork_start_height,
                ..
            } => (best_header_height, best_header_hash, fork_start_height),
            _ => {
                let chain = self.chain.read().unwrap();
                let tip = chain.get_tip().unwrap_or(None);
                if let Some(t) = tip {
                    (t.height as u32, t.block.header.hash(), None)
                } else {
                    (0, [0u8; 32], None)
                }
            }
        };
        drop(state);

        let chain = self.chain.read().unwrap();
        let mut header_map = self.header_height_map.lock().unwrap();

        let mut ts_window: VecDeque<crate::consensus::block::BlockHeader> =
            VecDeque::with_capacity(12);
        let mut cached_prev: Option<(crate::consensus::block::BlockHeader, u32)> = None;
        let mut headers_to_store: Vec<(crate::consensus::block::BlockHeader, u64)> =
            Vec::with_capacity(headers_msg.headers.len());

        for (idx, header) in headers_msg.headers.iter().enumerate() {
            let prev_hash = header.prev_block_hash;

            let (prev_header, prev_height) = if idx == 0 {
                log::debug!(
                    "[HEADERS] Trying prev_hash match: best_header_hash={}",
                    hex::encode(current_best_hash)
                );
                if prev_hash == current_best_hash {
                    if let Some(h) = chain
                        .block_store
                        .get_header(&prev_hash)
                        .map_err(|e| e.to_string())?
                    {
                        (h, current_best_height)
                    } else if prev_hash == [0u8; 32] {
                        return Err("Received genesis header?".to_string());
                    } else {
                        return Err(format!(
                            "Best header not found in DB: {}",
                            hex::encode(prev_hash)
                        ));
                    }
                } else {
                    log::debug!(
                        "[HEADERS] best_header_hash mismatch, looking up prev_hash {} in DB",
                        hex::encode(prev_hash)
                    );
                    match chain.block_store.get_header(&prev_hash) {
                        Ok(Some(prev_header_obj)) => {
                            let h_mem = chain.get_height_for_hash(&prev_hash);
                            let h_loc = h_mem.or_else(|| {
                                let last_locator = self.last_locator.lock().unwrap();
                                last_locator
                                    .iter()
                                    .find(|(lh, _)| lh == &prev_hash)
                                    .map(|(_, h)| *h)
                            });
                            let h_map = h_loc.or_else(|| {
                                self.peer_header_map
                                    .lock()
                                    .unwrap()
                                    .iter()
                                    .find(|(_, &ph)| ph == prev_hash)
                                    .map(|(&height, _)| height as u64)
                            });
                            log::debug!(
                                "[HEADERS] prev_hash height: mem={:?} loc={:?} map={:?} locator_len={}",
                                h_mem, h_loc, h_map,
                                self.last_locator.lock().unwrap().len()
                            );
                            let h = h_map.ok_or_else(|| {
                                format!(
                                    "prev_hash {} in DB but no height mapping",
                                    hex::encode(prev_hash)
                                )
                            })?;
                            (prev_header_obj, h as u32)
                        }
                        Ok(None) => {
                            log::debug!("[HEADERS] prev_hash not in DB, re-requesting headers");
                            drop(header_map);
                            drop(chain);
                            self.resend_getheaders(peer_id)?;
                            return Ok(());
                        }
                        Err(e) => {
                            return Err(format!("DB error looking up prev_hash: {}", e));
                        }
                    }
                }
            } else if prev_hash == current_best_hash {
                if let Some(cached) = cached_prev {
                    cached
                } else if let Some(h) = chain
                    .block_store
                    .get_header(&prev_hash)
                    .map_err(|e| e.to_string())?
                {
                    (h, current_best_height)
                } else {
                    return Err(format!(
                        "Best header not found in DB: {}",
                        hex::encode(prev_hash)
                    ));
                }
            } else if let Some(&h) = header_map.get(&prev_hash) {
                let header_obj = chain
                    .block_store
                    .get_header(&prev_hash)
                    .map_err(|e| e.to_string())?
                    .ok_or("Header in map but not DB")?;
                (header_obj, h)
            } else if let Some(h) = chain.get_height_for_hash(&prev_hash) {
                let block = chain.get_block(&prev_hash).ok_or("Prev block not found")?;
                (block.header, h as u32)
            } else {
                return Err(format!("Prev header not found: {}", hex::encode(prev_hash)));
            };

            // PoW check
            if !header
                .check_proof_of_work()
                .map_err(|_| "PoW check error")?
            {
                return Err(format!(
                    "Invalid PoW for header {}",
                    hex::encode(header.hash())
                ));
            }

            // Difficulty check
            validate_difficulty(Some(&prev_header), header, &chain.params)
                .map_err(|e| format!("Difficulty error: {:?}", e))?;

            // Timestamp check (MTP + future-time guard).
            {
                if ts_window.is_empty() {
                    ts_window.push_back(prev_header);
                    let mut ts_h = prev_height.saturating_sub(1);
                    while ts_window.len() < 11 {
                        match chain.block_store.get_header_by_height(ts_h as u64) {
                            Ok(Some(ph)) => {
                                ts_window.push_back(ph);
                                if ts_h == 0 {
                                    break;
                                }
                                ts_h = ts_h.saturating_sub(1);
                            }
                            _ => break,
                        }
                    }
                }
                let prev_headers_for_ts: Vec<_> = ts_window.iter().cloned().collect();
                validate_timestamp(header, &prev_headers_for_ts)
                    .map_err(|e| format!("Timestamp error: {:?}", e))?;
            }

            headers_to_store.push((*header, (prev_height + 1) as u64));

            let new_height = prev_height + 1;
            let new_hash = header.hash();

            header_map.insert(new_hash, new_height);

            // Fork detection
            if fork_start_height.is_none() {
                let canonical_height = chain.get_height() as u32;
                if new_height <= canonical_height {
                    match chain.block_store.get_block_by_height(new_height as u64) {
                        Ok(Some(canonical_block)) => {
                            if canonical_block.header.hash() != new_hash {
                                fork_start_height = Some(new_height);
                            }
                        }
                        _ => {
                            fork_start_height = Some(new_height);
                        }
                    }
                }
            }

            current_best_height = new_height;
            current_best_hash = new_hash;

            ts_window.push_front(*header);
            if ts_window.len() > 11 {
                ts_window.pop_back();
            }
            cached_prev = Some((*header, new_height));
        }

        // Batch write all validated headers.
        chain
            .block_store
            .store_headers_batch(&headers_to_store)
            .map_err(|e| e.to_string())?;

        // Persist best header.
        chain
            .state_store
            .store_best_header(current_best_height, &current_best_hash)?;

        drop(header_map);
        drop(chain);

        // Populate peer_header_map and peer_hash_to_height from this batch.
        {
            let mut peer_map = self.peer_header_map.lock().unwrap();
            let mut hash_map = self.peer_hash_to_height.lock().unwrap();
            for (header, height) in &headers_to_store {
                let hash = header.hash();
                peer_map.insert(*height as u32, hash);
                hash_map.insert(hash, *height as u32);
            }

            if let Some(&(_, first_height)) = headers_to_store.first() {
                if first_height > 0 {
                    let anchor_height = first_height as u32 - 1;
                    let anchor_hash = headers_msg.headers[0].prev_block_hash;
                    peer_map.entry(anchor_height).or_insert(anchor_hash);
                    hash_map.entry(anchor_hash).or_insert(anchor_height);
                }
            }
        }

        // Update state with latest header tip.
        {
            let mut state = self.sync_state.lock().unwrap();
            *state = SyncState::DownloadingHeaders {
                peer_id,
                best_header_height: current_best_height,
                best_header_hash: current_best_hash,
                fork_start_height,
            };
        }

        if headers_msg.headers.len() >= 2000 {
            let getheaders = GetHeadersMessage {
                version: 70015,
                block_locator: vec![current_best_hash],
                stop_hash: [0u8; 32],
            };
            let magic = self.peer_manager.magic();
            let msg = NetworkMessage::new(magic, "getheaders", getheaders.encode());
            self.peer_manager.send_to_peer(peer_id, &msg)?;
        } else {
            let first_missing = self.find_first_missing(current_best_height);
            let mut start = first_missing;
            while start > 0 {
                let parent_height = start - 1;
                let hash_opt = self
                    .peer_header_map
                    .lock()
                    .unwrap()
                    .get(&parent_height)
                    .copied();
                if let Some(hash) = hash_opt {
                    let chain = self.chain.read().unwrap();
                    match chain.block_store.get_block(&hash) {
                        Ok(Some(_)) => break,
                        _ => {
                            start = parent_height;
                        }
                    }
                } else {
                    let chain = self.chain.read().unwrap();
                    match chain.block_store.get_hash_by_height(parent_height as u64) {
                        Ok(Some(hash)) => {
                            let have_block =
                                chain.block_store.get_block(&hash).ok().flatten().is_some();
                            drop(chain);
                            self.peer_header_map
                                .lock()
                                .unwrap()
                                .insert(parent_height, hash);
                            self.peer_hash_to_height
                                .lock()
                                .unwrap()
                                .insert(hash, parent_height);
                            if have_block {
                                break;
                            } else {
                                start = parent_height;
                            }
                        }
                        _ => break,
                    }
                }
            }
            log::debug!(
                "SyncManager: headers done: first_missing={} start={} total_height={} fork_start={:?}",
                first_missing, start, current_best_height, fork_start_height
            );

            let local_height = self.get_local_height();
            let chain = self.chain.read().unwrap();
            let local_work = chain
                .state_store
                .get_tip()
                .ok()
                .flatten()
                .map(|t| t.cumulative_work)
                .unwrap_or(U256::from(0u64));
            drop(chain);

            let peer_work = self.compute_cumulative_work_for_tip(current_best_hash).ok();

            let should_download = if start > current_best_height {
                false
            } else if local_height < current_best_height {
                true
            } else if let Some(pw) = peer_work {
                pw > local_work || (pw == local_work && fork_start_height.is_some())
            } else {
                fork_start_height.is_some()
            };

            log::debug!(
                "SyncManager: headers done decision: local_height={} best_height={} start={} peer_work={:?} local_work={:?} fork_start={:?} download={}",
                local_height,
                current_best_height,
                start,
                peer_work.map(|w| w.0),
                local_work.0,
                fork_start_height,
                should_download
            );

            if should_download {
                let hashes = self.collect_pending_hashes(start, current_best_height);
                self.download_queue.lock().unwrap().enqueue_batch(hashes);

                let mut state = self.sync_state.lock().unwrap();
                *state = SyncState::DownloadingBlocks {
                    active_peers: vec![peer_id],
                    next_apply_height: start,
                    total_height: current_best_height,
                };
                drop(state);

                self.drain_to_peers_and_send(&[peer_id]);
            } else {
                let mut state = self.sync_state.lock().unwrap();
                *state = SyncState::Synced;
            }
        }

        Ok(())
    }

    /// Collect hashes from peer_header_map for heights `start..=end`, in order.
    fn collect_pending_hashes(&self, start: u32, end: u32) -> Vec<[u8; 32]> {
        let peer_map = self.peer_header_map.lock().unwrap();
        (start..=end)
            .filter_map(|h| peer_map.get(&h).copied())
            .collect()
    }

    // -----------------------------------------------------------------------
    // Serve headers to peers
    // -----------------------------------------------------------------------

    pub fn handle_getheaders(&self, peer_id: u64, msg: &GetHeadersMessage) -> Result<(), String> {
        log::debug!(
            "handle_getheaders called from peer {}, sending headers...",
            peer_id
        );
        let chain = self.chain.read().unwrap();

        let mut start_height = 0;
        let mut matched_at: Option<u64> = None;
        for hash in &msg.block_locator {
            let height_opt = chain.get_height_for_hash(hash).or_else(|| {
                chain
                    .block_store
                    .get_block_meta(hash)
                    .ok()
                    .flatten()
                    .map(|m| m.height)
            });
            if let Some(height) = height_opt {
                if let Ok(Some(canonical)) = chain.block_store.get_hash_by_height(height) {
                    if canonical == *hash {
                        start_height = height + 1;
                        matched_at = Some(height);
                        break;
                    }
                }
            }
        }
        log::debug!(
            "handle_getheaders: matched at height {:?}, sending from {}",
            matched_at,
            start_height
        );

        let mut headers = Vec::new();
        let mut height = start_height;
        while headers.len() < 2000 {
            let hash = match chain.block_store.get_hash_by_height(height) {
                Ok(Some(h)) => h,
                _ => break,
            };
            let header = match chain.block_store.get_header(&hash) {
                Ok(Some(h)) => h,
                _ => break,
            };
            headers.push(header);
            if msg.stop_hash != [0u8; 32] && hash == msg.stop_hash {
                break;
            }
            height += 1;
        }

        drop(chain);

        let headers_msg = HeadersMessage { headers };
        let magic = self.peer_manager.magic();
        let msg = NetworkMessage::new(magic, "headers", headers_msg.encode());

        self.peer_manager.send_to_peer(peer_id, &msg)?;

        Ok(())
    }

    fn resend_getheaders(&self, peer_id: u64) -> Result<(), String> {
        let chain = self.chain.read().unwrap();
        let locator_with_heights = chain.get_block_locator_with_heights();
        let locator: Vec<[u8; 32]> = locator_with_heights.iter().map(|(h, _)| *h).collect();
        drop(chain);

        {
            let mut last = self.last_locator.lock().unwrap();
            *last = locator_with_heights;
        }

        let getheaders = GetHeadersMessage {
            version: 70015,
            block_locator: locator,
            stop_hash: [0u8; 32],
        };

        let magic = self.peer_manager.magic();
        let msg = NetworkMessage::new(magic, "getheaders", getheaders.encode());
        self.peer_manager.send_to_peer(peer_id, &msg)?;
        Ok(())
    }
}
