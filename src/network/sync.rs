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
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Number of blocks to keep in-flight simultaneously during block download.
const DOWNLOAD_WINDOW: u32 = 64;

type PeerId = u64;
type LocatorEntry = ([u8; 32], u64);
type Locator = Vec<LocatorEntry>;

pub struct SyncManager {
    chain: Arc<RwLock<ChainState>>,
    peer_manager: Arc<PeerManager>,
    sync_state: Arc<Mutex<SyncState>>,
    header_height_map: Arc<Mutex<HashMap<[u8; 32], u32>>>,
    last_locator: Arc<Mutex<Locator>>,
    /// Peer's header chain from the current sync session: height → hash.
    /// Built incrementally from every incoming headers batch.  Never read from
    /// local DB — contains only what the remote peer sent us.
    peer_header_map: Arc<Mutex<HashMap<u32, [u8; 32]>>>,
    /// Reverse of peer_header_map for O(1) block lookup: hash → height.
    peer_hash_to_height: Arc<Mutex<HashMap<[u8; 32], u32>>>,
    /// Blocks received out-of-order, waiting for their predecessor to be applied.
    block_buffer: Arc<Mutex<HashMap<u32, Block>>>,
    /// Hashes currently requested via getdata but not yet received.
    inflight: Arc<Mutex<HashSet<[u8; 32]>>>,
    /// Timestamp of the last recheck_stalled_window execution (for rate limiting).
    last_recheck: Mutex<Instant>,
}

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
        peer_id: PeerId,
        /// Next block height we must apply before advancing.
        next_apply_height: u32,
        /// Highest block height in the peer's chain (sync target).
        total_height: u32,
    },
    Synced,
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
            inflight: Arc::new(Mutex::new(HashSet::new())),
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

    /// Called by the relay for every received block.
    ///
    /// Returns `None`  → not a tracked sync block; relay should handle via normal add_block.
    /// Returns `Some(applied)` → sync handled this block (applied or buffered).
    ///   Each entry is `(block, height)` for a block that was successfully added to the chain;
    ///   the relay should perform post-apply cleanup (mempool, peer-height) for each.
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

        // Remove from in-flight set (unconditional — keeps window accounting correct for
        // stale/duplicate arrivals too).
        self.inflight.lock().unwrap().remove(&block_hash);

        // Read the current download cursor (must not hold state lock while calling add_block).
        let (next_apply_height, peer_id, total_height) = {
            let state = self.sync_state.lock().unwrap();
            match *state {
                SyncState::DownloadingBlocks {
                    peer_id,
                    next_apply_height,
                    total_height,
                } => (next_apply_height, peer_id, total_height),
                _ => {
                    // Arrived before we transitioned to DownloadingBlocks — buffer it.
                    self.block_buffer.lock().unwrap().insert(height, block);
                    return Some(vec![]);
                }
            }
        };

        if height < next_apply_height {
            // Stale/duplicate — already applied; inflight already cleaned above.
            return Some(vec![]);
        }

        if height > next_apply_height {
            // Out-of-order — buffer it.
            let buf_len = {
                let mut buf = self.block_buffer.lock().unwrap();
                buf.insert(height, block);
                buf.len()
            };
            // If the buffer keeps growing the block at next_apply_height may have been
            // dropped by the network.  Force it back into the request window.
            if buf_len >= DOWNLOAD_WINDOW as usize {
                let stalled_hash = self
                    .peer_header_map
                    .lock()
                    .unwrap()
                    .get(&next_apply_height)
                    .copied();
                if let Some(h) = stalled_hash {
                    self.inflight.lock().unwrap().remove(&h);
                }
            }
            self.ensure_in_flight_window(peer_id, next_apply_height, total_height);
            return Some(vec![]);
        }

        // height == next_apply_height.
        // Hash-at-height guard: the block we received must be exactly the one the peer
        // advertised at this height (peer_header_map[height] == block_hash).
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
            // Re-add expected hash to window so it gets re-requested.
            if let Some(h) = expected_hash {
                self.inflight.lock().unwrap().remove(&h);
            }
            self.ensure_in_flight_window(peer_id, next_apply_height, total_height);
            return Some(vec![]);
        }

        // Prev-hash guard (height > 0): block.prev_hash must equal peer_header_map[height-1].
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
                    // Remove the expected hash from inflight so ensure_in_flight_window
                    // will re-request it.  next_apply_height is NOT advanced.
                    let expected_hash = self.peer_header_map.lock().unwrap().get(&height).copied();
                    if let Some(h) = expected_hash {
                        self.inflight.lock().unwrap().remove(&h);
                    }
                    self.ensure_in_flight_window(peer_id, next_apply_height, total_height);
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
            // chain write lock is released here — safe to send network messages below.

            match result {
                Ok(()) => {
                    applied.push((current_block, current_height));
                    current_height += 1;

                    if current_height > total_height {
                        let mut state = self.sync_state.lock().unwrap();
                        *state = SyncState::Synced;
                        log::info!("SyncManager: sync complete at height {}", total_height);
                        break;
                    }

                    {
                        let mut state = self.sync_state.lock().unwrap();
                        *state = SyncState::DownloadingBlocks {
                            peer_id,
                            next_apply_height: current_height,
                            total_height,
                        };
                    }

                    // Slide the download window forward.
                    self.ensure_in_flight_window(peer_id, current_height, total_height);

                    // Drain buffer: if the next block is already buffered, apply it now.
                    let next_block = self.block_buffer.lock().unwrap().remove(&current_height);
                    match next_block {
                        Some(b) => {
                            // Hash guard for buffer-drained blocks.
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
                            // Buffered blocks were never in inflight (only requested hashes
                            // go into inflight; buffering happens on receipt).
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

    /// Re-request the block at `next_apply_height` if the download appears stalled.
    /// Rate-limited to at most once every 10 seconds to prevent getdata spam under
    /// packet loss.  Safe to call from a timer loop or a peer-reconnect handler.
    pub fn recheck_stalled_window(&self) {
        const MIN_RECHECK_INTERVAL: Duration = Duration::from_secs(10);
        {
            let mut last = self.last_recheck.lock().unwrap();
            if last.elapsed() < MIN_RECHECK_INTERVAL {
                return;
            }
            *last = Instant::now();
        }

        let (peer_id, next_apply_height, total_height) = {
            let state = self.sync_state.lock().unwrap();
            match *state {
                SyncState::DownloadingBlocks {
                    peer_id,
                    next_apply_height,
                    total_height,
                } => (peer_id, next_apply_height, total_height),
                _ => return,
            }
        };
        // Clear the stalled height from inflight so ensure_in_flight_window will re-request it.
        let stalled_hash = self
            .peer_header_map
            .lock()
            .unwrap()
            .get(&next_apply_height)
            .copied();
        if let Some(h) = stalled_hash {
            self.inflight.lock().unwrap().remove(&h);
        }
        self.ensure_in_flight_window(peer_id, next_apply_height, total_height);
    }

    /// Spawn a background thread that calls `recheck_stalled_window` every 30 seconds.
    /// Uses a weak reference so the thread exits automatically when the manager is dropped.
    /// Call once, immediately after constructing the `SyncManager`.
    pub fn start_stall_checker(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("sync-stall-checker".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(30));
                match weak.upgrade() {
                    Some(mgr) => mgr.recheck_stalled_window(),
                    None => break, // SyncManager has been dropped; exit thread.
                }
            })
            .expect("failed to spawn sync-stall-checker thread");
    }

    /// Fill the download window [next_apply_height .. next_apply_height+W) by
    /// sending getdata for any peer-chain blocks not already in-flight.
    fn ensure_in_flight_window(&self, peer_id: u64, next_apply_height: u32, total_height: u32) {
        let mut inflight = self.inflight.lock().unwrap();
        let peer_map = self.peer_header_map.lock().unwrap();

        let window_end = std::cmp::min(next_apply_height + DOWNLOAD_WINDOW, total_height + 1);
        let mut pending: Vec<[u8; 32]> = Vec::new();

        for h in next_apply_height..window_end {
            if let Some(&hash) = peer_map.get(&h) {
                if !inflight.contains(&hash) {
                    inflight.insert(hash);
                    pending.push(hash);
                }
            }
        }

        drop(inflight);
        drop(peer_map);

        if pending.is_empty() {
            return;
        }

        let magic = self.peer_manager.magic();
        let getdata = GetDataMessage {
            inventory: pending
                .iter()
                .map(|hash| InvVector {
                    inv_type: INV_BLOCK,
                    hash: *hash,
                })
                .collect(),
        };
        let msg = NetworkMessage::new(magic, "getdata", getdata.encode());
        let _ = self.peer_manager.send_to_peer(peer_id, &msg);
        log::debug!(
            "SyncManager: window [{}..{}] → {} blocks requested from peer {}",
            next_apply_height,
            window_end - 1,
            pending.len(),
            peer_id
        );
    }

    /// Walk peer_header_map (sorted by height) and return the first height where
    /// we don't already have the block in our DB.  Returns `total_height + 1` if
    /// we have every block (nothing to download).
    fn find_first_missing(&self, total_height: u32) -> u32 {
        let peer_map = self.peer_header_map.lock().unwrap();
        let chain = self.chain.read().unwrap();

        let mut sorted: Vec<u32> = peer_map.keys().cloned().collect();
        sorted.sort_unstable();

        for h in sorted {
            if let Some(&hash) = peer_map.get(&h) {
                match chain.block_store.get_block(&hash) {
                    Ok(Some(_)) => {} // already have it
                    _ => return h,
                }
            }
        }

        total_height + 1 // have everything
    }

    /// Clear all per-session sync state before starting a new sync.
    fn reset_sync_state(&self) {
        self.peer_header_map.lock().unwrap().clear();
        self.peer_hash_to_height.lock().unwrap().clear();
        self.block_buffer.lock().unwrap().clear();
        self.inflight.lock().unwrap().clear();
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

    // Legacy notification — kept for API compatibility; no longer drives block downloads.
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

    // Start sync process with a peer
    pub fn start_sync(&self, peer_id: u64) -> Result<(), String> {
        log::debug!("SyncManager: Starting sync check with peer {}", peer_id);

        // Always skip if already downloading headers.
        {
            let state = self.sync_state.lock().unwrap();
            if matches!(*state, SyncState::DownloadingHeaders { .. }) {
                log::debug!("SyncManager: Already downloading headers, skipping duplicate sync.");
                return Ok(());
            }
        }

        // Clear state from any prior sync session.
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

        // Announce our local tip so the peer can discover our chain and request our headers.
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

    // Handle received headers
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
                        let mut state = self.sync_state.lock().unwrap();
                        *state = SyncState::DownloadingBlocks {
                            peer_id: sync_peer_id,
                            next_apply_height: start,
                            total_height: best_height,
                        };
                        drop(state);

                        self.ensure_in_flight_window(sync_peer_id, start, best_height);
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
                                // Check peer_header_map (heights we've already received).
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

            // Fork detection: first height where peer's hash differs from our canonical block.
            // Use block_store.get_block_by_height (CF_BLOCK_INDEX, written only by add_block)
            // NOT get_header_at_height (CF_HEADERS, overwritten by store_headers_batch).
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
        // This is the authoritative source for fork-chain heights — never use local DB indexes.
        {
            let mut peer_map = self.peer_header_map.lock().unwrap();
            let mut hash_map = self.peer_hash_to_height.lock().unwrap();
            for (header, height) in &headers_to_store {
                let hash = header.hash();
                peer_map.insert(*height as u32, hash);
                hash_map.insert(hash, *height as u32);
            }

            // Also record the anchor block — the parent of the first received header.
            // The anchor is the common-ancestor block that the peer's fork chain builds on.
            // If it exists only in CF_HEADERS (e.g., from a prior header-only sync) but
            // NOT in CF_BLOCKS, find_first_missing will flag it as missing and download it
            // from the peer before attempting to apply the fork chain.  Without this,
            // add_block for the first fork block fails with OrphanBlock because the parent
            // cannot be found in self.blocks or CF_BLOCKS.
            if let Some(&(_, first_height)) = headers_to_store.first() {
                if first_height > 0 {
                    let anchor_height = first_height as u32 - 1;
                    let anchor_hash = headers_msg.headers[0].prev_block_hash;
                    // or_insert is a no-op on subsequent batches where this height is
                    // already populated from the previous batch's final entry.
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
            // More headers to fetch — continue header sync.
            let getheaders = GetHeadersMessage {
                version: 70015,
                block_locator: vec![current_best_hash],
                stop_hash: [0u8; 32],
            };
            let magic = self.peer_manager.magic();
            let msg = NetworkMessage::new(magic, "getheaders", getheaders.encode());
            self.peer_manager.send_to_peer(peer_id, &msg)?;
        } else {
            // Headers complete — determine the first block we actually need to download.
            // Use hash-existence check against peer_header_map (peer's heights), never
            // local height indexes (those reflect our canonical chain, not the fork).
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
                    // Use CF_BLOCK_INDEX (canonical chain) to get the parent hash.
                    // CF_HEADERS can be stale from prior sync sessions and must not
                    // be used here — a wrong hash would cause prev_hash mismatches.
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
                let mut state = self.sync_state.lock().unwrap();
                *state = SyncState::DownloadingBlocks {
                    peer_id,
                    next_apply_height: start,
                    total_height: current_best_height,
                };
                drop(state);

                self.ensure_in_flight_window(peer_id, start, current_best_height);
            } else {
                let mut state = self.sync_state.lock().unwrap();
                *state = SyncState::Synced;
            }
        }

        Ok(())
    }

    // Handle getheaders request — serve our canonical chain headers to the peer.
    pub fn handle_getheaders(&self, peer_id: u64, msg: &GetHeadersMessage) -> Result<(), String> {
        log::debug!(
            "handle_getheaders called from peer {}, sending headers...",
            peer_id
        );
        let chain = self.chain.read().unwrap();

        // Find common ancestor from locator.
        // Use only self.blocks (the in-memory canonical chain index) for height lookup.
        // get_block_meta would return a height for ANY stored block including side-chain
        // blocks, so using it here would allow a contaminated CF_BLOCK_INDEX entry to
        // pass the canonical check incorrectly.  self.blocks is authoritative: it is
        // populated exclusively from the canonical tip walk in recover_from_storage.
        let mut start_height = 0;
        let mut matched_at: Option<u64> = None;
        for hash in &msg.block_locator {
            // Check LRU cache first; fall back to block_meta in DB for blocks
            // outside the cache window (deep forks, exponential locator steps).
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

        // Serve canonical headers by looking up each height via CF_BLOCK_INDEX first,
        // then fetching the header by hash.  get_header_at_height / get_header_by_height
        // prefers the CF_HEADERS height key (hh: prefix) which is overwritten by
        // store_headers_batch during every peer sync, so using it here would serve
        // stale headers from a prior sync session instead of the canonical chain.
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
