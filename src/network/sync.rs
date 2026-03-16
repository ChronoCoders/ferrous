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

type PeerId = u64;
type LocatorEntry = ([u8; 32], u64);
type Locator = Vec<LocatorEntry>;

pub struct SyncManager {
    chain: Arc<RwLock<ChainState>>,
    peer_manager: Arc<PeerManager>,
    sync_state: Arc<Mutex<SyncState>>,
    header_height_map: Arc<Mutex<HashMap<[u8; 32], u32>>>,
    last_locator: Arc<Mutex<Locator>>,
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
        next_expected_height: u32,
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

    pub fn block_received(&self, hash: [u8; 32]) {
        let chain = self.chain.read().unwrap();
        // Use the received block's own height rather than the canonical tip height.
        // Fork/side-chain blocks don't advance the canonical tip, so get_height() would
        // return the pre-fork canonical height and skip past all fork blocks.
        let block_height = chain
            .get_height_for_hash(&hash)
            .unwrap_or_else(|| chain.get_height()) as u32;
        drop(chain);

        let mut state = self.sync_state.lock().unwrap();
        if let SyncState::DownloadingBlocks {
            next_expected_height,
        } = *state
        {
            if block_height >= next_expected_height {
                let next_height = block_height + 1;
                *state = SyncState::DownloadingBlocks {
                    next_expected_height: next_height,
                };
                drop(state); // Drop lock before requesting

                let peer_id = match self.select_best_peer() {
                    Some(p) => p,
                    None => return,
                };

                let chain = self.chain.read().unwrap();
                let best_header_height = chain
                    .state_store
                    .load_best_header()
                    .unwrap_or(None)
                    .map(|(h, _)| h)
                    .unwrap_or(0);

                if next_height > best_header_height {
                    drop(chain);
                    let mut state = self.sync_state.lock().unwrap();
                    *state = SyncState::Synced;
                    return;
                }

                let mut headers = Vec::new();
                let end_height = std::cmp::min(next_height + 500, best_header_height);
                for h in next_height..=end_height {
                    if let Some(header) = chain.get_header_at_height(h as u64) {
                        headers.push(header);
                    } else {
                        break;
                    }
                }
                drop(chain);

                if !headers.is_empty() {
                    let _ = self.request_blocks_from_headers(peer_id, &headers);
                }
            }
        }
    }

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
        println!("SyncManager: Starting sync check with peer {}", peer_id);

        let chain = self.chain.read().unwrap();
        let our_height = chain.get_height() as u32;

        // Determine current best header (from DB or memory)
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

        // Use the better of local chain tip or best header
        let (start_height, start_hash) = if best_header_height > our_height {
            (best_header_height, best_header_hash)
        } else {
            // Use tip
            let tip = chain.get_tip().unwrap_or(None);
            if let Some(t) = tip {
                (t.height as u32, t.block.header.hash())
            } else {
                (0, [0u8; 32])
            }
        };

        drop(chain);

        let peer_height = self
            .peer_manager
            .get_peer_start_height(peer_id)
            .unwrap_or(0);
        println!(
            "SyncManager: Local height: {}, Best Header: {}, Peer {} height: {}",
            our_height, start_height, peer_id, peer_height
        );

        // Always request headers regardless of the peer's reported height from the VERSION
        // handshake — that value is stale within seconds of connection. Skipping based on
        // height alone prevents fork detection when both nodes are at similar heights.
        // Instead, skip only when a headers download is already in flight.
        {
            let state = self.sync_state.lock().unwrap();
            if matches!(*state, SyncState::DownloadingHeaders { .. }) {
                println!("SyncManager: Already downloading headers, skipping duplicate sync.");
                return Ok(());
            }
        }

        let chain = self.chain.read().unwrap();
        let locator_with_heights = chain.get_block_locator_with_heights();
        let locator: Vec<[u8; 32]> = locator_with_heights.iter().map(|(h, _)| *h).collect();
        drop(chain);

        {
            let mut last = self.last_locator.lock().unwrap();
            *last = locator_with_heights;
        }

        println!("SyncManager: Requesting headers from peer {}", peer_id);

        // Update state to DownloadingHeaders
        let mut state = self.sync_state.lock().unwrap();
        *state = SyncState::DownloadingHeaders {
            peer_id,
            best_header_height: start_height,
            best_header_hash: start_hash,
            fork_start_height: None,
        };
        drop(state);

        // Request headers
        let getheaders = GetHeadersMessage {
            version: 70015,
            block_locator: locator,
            stop_hash: [0u8; 32], // Get all headers
        };

        let magic = self.peer_manager.magic();
        let msg = NetworkMessage::new(magic, "getheaders", getheaders.encode());

        self.peer_manager.send_to_peer(peer_id, &msg)?;

        // Announce our local chain tip so the peer can request our headers if we have
        // more cumulative work.  This ensures both sides discover the heavier chain and
        // let add_block's cumulative-work comparison trigger the reorg — rather than
        // only the connecting side ever downloading the peer's chain.
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
        // Similar to start_sync but force request
        self.start_sync(peer_id)
    }

    // Handle received headers
    pub fn handle_headers(&self, peer_id: u64, headers_msg: &HeadersMessage) -> Result<(), String> {
        println!(
            "SyncManager: Received {} headers from peer {}",
            headers_msg.headers.len(),
            peer_id
        );

        if headers_msg.headers.is_empty() {
            // Check if we need to download blocks (Headers are synced, but blocks might not be)
            let local_height = self.get_local_height();
            let state = self.sync_state.lock().unwrap();

            // Get best header height from state or DB
            let (best_height_opt, empty_fork_start) = match *state {
                SyncState::DownloadingHeaders {
                    best_header_height,
                    fork_start_height,
                    ..
                } => (Some(best_header_height), fork_start_height),
                _ => (None, None),
            };
            drop(state);

            let best_height = if let Some(h) = best_height_opt {
                h
            } else {
                let chain = self.chain.read().unwrap();
                chain
                    .state_store
                    .load_best_header()
                    .unwrap_or(None)
                    .map(|(h, _)| h)
                    .unwrap_or(0)
            };

            if local_height < best_height {
                // We have headers but need blocks.  Use the fork start as the download
                // cursor so we fetch from the actual divergence point, not canonical tip+1.
                let first_missing = empty_fork_start.unwrap_or(local_height + 1);
                let mut state = self.sync_state.lock().unwrap();
                *state = SyncState::DownloadingBlocks {
                    next_expected_height: first_missing,
                };
                drop(state);

                // Fetch next batch of headers from DB to request blocks
                let chain = self.chain.read().unwrap();
                let mut headers = Vec::new();
                // Limit to 500 blocks per batch
                let end_height = std::cmp::min(first_missing + 499, best_height);
                for h in first_missing..=end_height {
                    if let Some(header) = chain.get_header_at_height(h as u64) {
                        headers.push(header);
                    } else {
                        break;
                    }
                }
                drop(chain);

                if !headers.is_empty() {
                    println!(
                        "SyncManager: Headers synced, downloading {} blocks from height {}",
                        headers.len(),
                        local_height + 1
                    );
                    self.request_blocks_from_headers(peer_id, &headers)?;
                    return Ok(());
                }
            }

            let mut state = self.sync_state.lock().unwrap();
            *state = SyncState::Synced;
            return Ok(());
        }

        let state = self.sync_state.lock().unwrap();

        // Extract current best header info from state if we are in DownloadingHeaders
        let (mut current_best_height, mut current_best_hash, mut fork_start_height) = match *state {
            SyncState::DownloadingHeaders {
                best_header_height,
                best_header_hash,
                fork_start_height,
                ..
            } => (best_header_height, best_header_hash, fork_start_height),
            _ => {
                // If not in DownloadingHeaders, maybe we are Idle or Synced and received unsolicited headers?
                // Or legacy state?
                // For Phase 1, we assume we initiated it via start_sync.
                // We'll load from DB as fallback.
                let chain = self.chain.read().unwrap();
                let tip = chain.get_tip().unwrap_or(None);
                if let Some(t) = tip {
                    (t.height as u32, t.block.header.hash(), None)
                } else {
                    (0, [0u8; 32], None)
                }
            }
        };
        drop(state); // Drop lock during validation loop to avoid long hold? No, we need chain lock.

        let chain = self.chain.read().unwrap();
        let mut header_map = self.header_height_map.lock().unwrap();

        // Sliding window of up to 11 recent headers used for MTP timestamp validation.
        // Seeded from DB on the first header of the batch; updated in O(1) per iteration
        // thereafter — eliminates 11 DB reads per header (≈22 000 reads/batch).
        let mut ts_window: VecDeque<crate::consensus::block::BlockHeader> =
            VecDeque::with_capacity(12);

        // Cached prev_header from the previous iteration (idx > 0 sequential path).
        // Avoids one DB round-trip per header once the chain is connected.
        let mut cached_prev: Option<(crate::consensus::block::BlockHeader, u32)> = None;

        // Accumulate validated headers for a single-batch DB write at the end of the loop.
        // This collapses 2 000 individual WriteBatch+fsync calls into one, eliminating the
        // dominant I/O cost on slow-disk nodes (~240 ms/write → minutes per batch).
        let mut headers_to_store: Vec<(crate::consensus::block::BlockHeader, u64)> =
            Vec::with_capacity(headers_msg.headers.len());

        for (idx, header) in headers_msg.headers.iter().enumerate() {
            let prev_hash = header.prev_block_hash;

            // 1. Determine prev_header and prev_height
            let (prev_header, prev_height) = if idx == 0 {
                println!(
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
                    // prev_hash doesn't match our current best header.  Directly look up
                    // prev_hash in the DB — this handles divergent chains where the peer
                    // sends headers anchored at a common ancestor behind our tip.
                    println!(
                        "[HEADERS] best_header_hash mismatch, looking up prev_hash {} in DB",
                        hex::encode(prev_hash)
                    );
                    match chain.block_store.get_header(&prev_hash) {
                        Ok(Some(prev_header_obj)) => {
                            // get_height_for_hash only covers in-memory (add_block) blocks.
                            // For headers-only entries (stored via store_header_at_height
                            // during a previous partial sync), fall back to:
                            //   1. our saved locator — the common ancestor is always one of
                            //      our locator hashes, stored with its height; or
                            //   2. block_meta — populated for fully-downloaded side-chain blocks.
                            let h = chain
                                .get_height_for_hash(&prev_hash)
                                .or_else(|| {
                                    let last_locator = self.last_locator.lock().unwrap();
                                    last_locator
                                        .iter()
                                        .find(|(lh, _)| lh == &prev_hash)
                                        .map(|(_, h)| *h)
                                })
                                .or_else(|| {
                                    chain
                                        .block_store
                                        .get_block_meta(&prev_hash)
                                        .ok()
                                        .flatten()
                                        .map(|m| m.height)
                                })
                                .ok_or_else(|| {
                                    format!(
                                        "prev_hash {} in DB but no height mapping",
                                        hex::encode(prev_hash)
                                    )
                                })?;
                            // Accept the header as the anchor for this batch.  The header
                            // is already in our DB (it passed PoW validation when stored),
                            // and each subsequent header in the batch is validated below.
                            // add_block handles the cumulative-work comparison and reorg
                            // when the downloaded blocks are applied — no need to enforce
                            // canonical-chain membership here.
                            (prev_header_obj, h as u32)
                        }
                        Ok(None) => {
                            println!("[HEADERS] prev_hash not in DB, re-requesting headers");
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
                // Subsequent headers should match previous best (which we updated).
                // Use the in-memory cache to avoid a DB round-trip.
                if let Some(cached) = cached_prev {
                    cached
                } else {
                    // Fallback to DB on the rare first-iteration mismatch path.
                    if let Some(h) = chain
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
                }
            } else {
                // Check map (should be rare if sequential)
                if let Some(&h) = header_map.get(&prev_hash) {
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
                }
            };

            // 2. PoW Check
            if !header
                .check_proof_of_work()
                .map_err(|_| "PoW check error")?
            {
                return Err(format!(
                    "Invalid PoW for header {}",
                    hex::encode(header.hash())
                ));
            }

            // 3. Difficulty Check
            validate_difficulty(Some(&prev_header), header, &chain.params)
                .map_err(|e| format!("Difficulty error: {:?}", e))?;

            // 4. Timestamp check (MTP + future-time guard).
            // Seed the sliding window from DB on the very first header of the batch
            // (≤11 reads, done once).  Subsequent headers are validated in O(1) using
            // the in-memory window, avoiding ≈11 DB reads per header.
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

            // 6. Collect for batch write (committed once after the loop).
            headers_to_store.push((*header, (prev_height + 1) as u64));

            // 7. Update tracking
            let new_height = prev_height + 1;
            let new_hash = header.hash();

            header_map.insert(new_hash, new_height);

            // Detect the fork start: the first height where the peer's header hash
            // differs from OUR canonical block at that same height.
            //
            // IMPORTANT: use block_store.get_block_by_height (CF_BLOCK_INDEX, updated
            // only by add_block), NOT get_header_at_height (CF_HEADERS, which may have
            // been overwritten by a previous peer header sync).  Using CF_HEADERS would
            // compare peer-vs-peer and never detect the fork.
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
                            // Canonical block missing at this height — treat as fork start.
                            fork_start_height = Some(new_height);
                        }
                    }
                }
            }

            // Update best if we extended the chain
            // Note: We are following the peer's chain.
            // In DownloadingHeaders, we track *this peer's* best header.
            current_best_height = new_height;
            current_best_hash = new_hash;

            // Slide the window forward: new header becomes the most-recent entry.
            ts_window.push_front(*header);
            if ts_window.len() > 11 {
                ts_window.pop_back();
            }

            // Cache this header so the next iteration can use it without a DB read.
            cached_prev = Some((*header, new_height));
        }

        // Batch write all validated headers in one atomic commit (no WAL).
        chain
            .block_store
            .store_headers_batch(&headers_to_store)
            .map_err(|e| e.to_string())?;

        // Persist best header
        chain
            .state_store
            .store_best_header(current_best_height, &current_best_hash)?;

        drop(header_map);
        drop(chain);

        // Update state
        let mut state = self.sync_state.lock().unwrap();
        *state = SyncState::DownloadingHeaders {
            peer_id,
            best_header_height: current_best_height,
            best_header_hash: current_best_hash,
            fork_start_height,
        };

        // Check batch size
        if headers_msg.headers.len() >= 2000 {
            // Continue downloading
            drop(state); // Release lock

            // Send next getheaders
            let chain = self.chain.read().unwrap();
            let locator = vec![current_best_hash]; // Use new best hash as locator
            drop(chain);

            let getheaders = GetHeadersMessage {
                version: 70015,
                block_locator: locator,
                stop_hash: [0u8; 32],
            };
            let magic = self.peer_manager.magic();
            let msg = NetworkMessage::new(magic, "getheaders", getheaders.encode());
            self.peer_manager.send_to_peer(peer_id, &msg)?;
        } else {
            // Headers complete — use fork_start_height as the download cursor so that
            // fork-chain blocks are fetched from the actual divergence point.  Without
            // this, first_missing = canonical_tip + 1 which is past the fork: block at
            // that height has a parent that doesn't exist in the peer's DB → OrphanBlock.
            let first_missing = fork_start_height.unwrap_or_else(|| self.get_local_height() + 1);
            println!(
                "SyncManager: fork_start_height={:?} first_missing={} current_best_height={}",
                fork_start_height, first_missing, current_best_height
            );

            *state = SyncState::DownloadingBlocks {
                next_expected_height: first_missing,
            };
            drop(state);

            // Read headers stored in CF_HEADERS (which now hold the peer's fork chain)
            // starting at first_missing so we request the correct blocks.
            let chain = self.chain.read().unwrap();
            let mut headers_to_dl: Vec<crate::consensus::block::BlockHeader> = Vec::new();
            let end_height = std::cmp::min(first_missing + 499, current_best_height);
            for h in first_missing..=end_height {
                if let Some(hdr) = chain.get_header_at_height(h as u64) {
                    headers_to_dl.push(hdr);
                } else {
                    break;
                }
            }
            drop(chain);

            if !headers_to_dl.is_empty() {
                self.request_blocks_from_headers(peer_id, &headers_to_dl)?;
            } else {
                self.request_blocks_from_headers(peer_id, &headers_msg.headers)?;
            }
        }

        Ok(())
    }

    // Legacy block downloader (adapted to work with new flow)
    fn request_blocks_from_headers(
        &self,
        peer_id: u64,
        headers: &[crate::consensus::block::BlockHeader],
    ) -> Result<(), String> {
        let chain = self.chain.read().unwrap();
        let mut pending = Vec::new();

        for header in headers {
            let hash = header.hash();
            if chain.get_block(&hash).is_none() {
                pending.push(hash);
            }
            if pending.len() >= 500 {
                break;
            }
        }
        drop(chain);

        if !pending.is_empty() {
            // Send GetData
            let inv_vecs: Vec<InvVector> = pending
                .iter()
                .map(|hash| InvVector {
                    inv_type: INV_BLOCK,
                    hash: *hash,
                })
                .collect();

            let getdata = GetDataMessage {
                inventory: inv_vecs,
            };

            let magic = self.peer_manager.magic();
            let msg = NetworkMessage::new(magic, "getdata", getdata.encode());
            self.peer_manager.send_to_peer(peer_id, &msg)?;
            println!(
                "SyncManager: Sent getdata for {} blocks to peer {}",
                pending.len(),
                peer_id
            );

            // We are already in DownloadingBlocks state or transitioning to it.
            // We don't update state here as we rely on block_received to track progress.
        } else {
            // If no blocks needed from this batch, maybe we are synced?
            // Or maybe we need to check next batch?
            // For now, assume Synced if no pending.
            let mut state = self.sync_state.lock().unwrap();
            *state = SyncState::Synced;
        }

        Ok(())
    }

    // Handle getheaders request (Unchanged logic, just ensure it works)
    pub fn handle_getheaders(&self, peer_id: u64, msg: &GetHeadersMessage) -> Result<(), String> {
        println!(
            "handle_getheaders called from peer {}, sending headers...",
            peer_id
        );
        let chain = self.chain.read().unwrap();

        // Find common ancestor from locator
        let mut start_height = 0;
        for hash in &msg.block_locator {
            if let Some(height) = chain.get_height_for_hash(hash) {
                if let Some(active_block) = chain.get_block_by_height(height) {
                    if active_block.hash() == *hash {
                        start_height = height + 1;
                        break;
                    }
                }
            }
        }

        let mut headers = Vec::new();
        let mut height = start_height;
        loop {
            if headers.len() >= 2000 {
                break;
            }

            if let Some(header) = chain.get_header_at_height(height) {
                let hash = header.hash();
                headers.push(header);

                if msg.stop_hash != [0u8; 32] && hash == msg.stop_hash {
                    break;
                }
                height += 1;
            } else {
                break;
            }
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
