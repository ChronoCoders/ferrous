use crate::consensus::chain::ChainState;
use crate::consensus::difficulty::validate_difficulty;
use crate::consensus::validation::validate_timestamp;
use crate::network::manager::PeerManager;
use crate::network::message::NetworkMessage;
use crate::network::protocol::{
    GetDataMessage, GetHeadersMessage, HeadersMessage, InvVector, INV_BLOCK,
};
use crate::primitives::serialize::Encode;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

type PeerId = u64;
type LocatorEntry = ([u8; 32], u64);
type Locator = Vec<LocatorEntry>;

pub struct SyncManager {
    chain: Arc<Mutex<ChainState>>,
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
    },
    DownloadingBlocks {
        next_expected_height: u32,
    },
    Synced,
}

impl SyncManager {
    pub fn new(chain: Arc<Mutex<ChainState>>, peer_manager: Arc<PeerManager>) -> Self {
        Self {
            chain,
            peer_manager,
            sync_state: Arc::new(Mutex::new(SyncState::Idle)),
            header_height_map: Arc::new(Mutex::new(HashMap::new())),
            last_locator: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn get_local_height(&self) -> u32 {
        let chain = self.chain.lock().unwrap();
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

    pub fn block_received(&self, _hash: [u8; 32]) {
        let chain = self.chain.lock().unwrap();
        let current_height = chain.get_height() as u32;
        drop(chain);

        let mut state = self.sync_state.lock().unwrap();
        if let SyncState::DownloadingBlocks {
            next_expected_height,
        } = *state
        {
            if current_height >= next_expected_height {
                let next_height = current_height + 1;
                *state = SyncState::DownloadingBlocks {
                    next_expected_height: next_height,
                };
                drop(state); // Drop lock before requesting

                let peer_id = match self.select_best_peer() {
                    Some(p) => p,
                    None => return,
                };

                let chain = self.chain.lock().unwrap();
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

        let chain = self.chain.lock().unwrap();
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

        let chain = self.chain.lock().unwrap();
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
            let best_height_opt = match *state {
                SyncState::DownloadingHeaders {
                    best_header_height, ..
                } => Some(best_header_height),
                _ => None,
            };
            drop(state);

            let best_height = if let Some(h) = best_height_opt {
                h
            } else {
                let chain = self.chain.lock().unwrap();
                chain
                    .state_store
                    .load_best_header()
                    .unwrap_or(None)
                    .map(|(h, _)| h)
                    .unwrap_or(0)
            };

            if local_height < best_height {
                // We have headers but need blocks.
                let mut state = self.sync_state.lock().unwrap();
                *state = SyncState::DownloadingBlocks {
                    next_expected_height: local_height + 1,
                };
                drop(state);

                // Fetch next batch of headers from DB to request blocks
                let chain = self.chain.lock().unwrap();
                let mut headers = Vec::new();
                // Limit to 500 blocks per batch
                let end_height = std::cmp::min(local_height + 500, best_height);
                for h in (local_height + 1)..=end_height {
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
        let (mut current_best_height, mut current_best_hash) = match *state {
            SyncState::DownloadingHeaders {
                best_header_height,
                best_header_hash,
                ..
            } => (best_header_height, best_header_hash),
            _ => {
                // If not in DownloadingHeaders, maybe we are Idle or Synced and received unsolicited headers?
                // Or legacy state?
                // For Phase 1, we assume we initiated it via start_sync.
                // We'll load from DB as fallback.
                let chain = self.chain.lock().unwrap();
                let tip = chain.get_tip().unwrap_or(None);
                if let Some(t) = tip {
                    (t.height as u32, t.block.header.hash())
                } else {
                    (0, [0u8; 32])
                }
            }
        };
        drop(state); // Drop lock during validation loop to avoid long hold? No, we need chain lock.

        let chain = self.chain.lock().unwrap();
        let mut header_map = self.header_height_map.lock().unwrap();

        // Sliding window of up to 11 recent headers used for MTP timestamp validation.
        // Seeded from DB on the first header of the batch; updated in O(1) per iteration
        // thereafter — eliminates 11 DB reads per header (≈22 000 reads/batch).
        let mut ts_window: VecDeque<crate::consensus::block::BlockHeader> =
            VecDeque::with_capacity(12);

        // Cached prev_header from the previous iteration (idx > 0 sequential path).
        // Avoids one DB round-trip per header once the chain is connected.
        let mut cached_prev: Option<(crate::consensus::block::BlockHeader, u32)> = None;

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
                            let h =
                                chain.get_height_for_hash(&prev_hash).ok_or_else(|| {
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

            // 6. Store
            chain
                .store_header_only_at_height(header, (prev_height + 1) as u64)
                .map_err(|e| e.to_string())?;

            // 7. Update tracking
            let new_height = prev_height + 1;
            let new_hash = header.hash();

            header_map.insert(new_hash, new_height);

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
        };

        // Check batch size
        if headers_msg.headers.len() >= 2000 {
            // Continue downloading
            drop(state); // Release lock

            // Send next getheaders
            let chain = self.chain.lock().unwrap();
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
            // Headers complete
            // Transition to DownloadingBlocks (Phase 2 marker)
            let first_missing = self.get_local_height() + 1;

            *state = SyncState::DownloadingBlocks {
                next_expected_height: first_missing,
            };
            drop(state);

            // Trigger legacy block download (Phase 1 fallback)
            // We need to fetch the list of blocks to download.
            // Since we have headers, we can walk from first_missing to current_best_height.
            // But request_blocks_from_headers (legacy) expects a list of headers.
            // We don't have the list of *all* headers in memory (we just processed a batch).
            // But we can construct a request.
            // Actually, existing request_blocks_from_headers takes `&[BlockHeader]`.
            // We can pass the `headers_msg.headers` we just received?
            // If we are catching up from scratch, we received 2000 headers.
            // We should request blocks for those 2000 headers.
            // Yes, passing the current batch to legacy downloader is a good strategy for now.

            self.request_blocks_from_headers(peer_id, &headers_msg.headers)?;
        }

        Ok(())
    }

    // Legacy block downloader (adapted to work with new flow)
    fn request_blocks_from_headers(
        &self,
        peer_id: u64,
        headers: &[crate::consensus::block::BlockHeader],
    ) -> Result<(), String> {
        let chain = self.chain.lock().unwrap();
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
        let chain = self.chain.lock().unwrap();

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
        let chain = self.chain.lock().unwrap();
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
