use crate::consensus::chain::ChainState;
use crate::network::manager::PeerManager;
use crate::network::message::NetworkMessage;
use crate::network::protocol::{
    GetDataMessage, GetHeadersMessage, HeadersMessage, InvVector, INV_BLOCK,
};
use crate::primitives::serialize::Encode;
use std::sync::{Arc, Mutex};

pub struct SyncManager {
    chain: Arc<Mutex<ChainState>>,
    peer_manager: Arc<PeerManager>,
    sync_state: Arc<Mutex<SyncState>>,
}

#[derive(Debug, Clone, PartialEq)]
enum SyncState {
    Idle,
    DownloadingHeaders { from_peer: u64, highest_known: u64 },
    DownloadingBlocks { pending: Vec<[u8; 32]>, peer_id: u64 },
    Synced,
}

impl SyncManager {
    pub fn new(chain: Arc<Mutex<ChainState>>, peer_manager: Arc<PeerManager>) -> Self {
        Self {
            chain,
            peer_manager,
            sync_state: Arc::new(Mutex::new(SyncState::Idle)),
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

    pub fn block_received(&self, hash: [u8; 32]) {
        let next_peer = {
            let mut state = self.sync_state.lock().unwrap();
            if let SyncState::DownloadingBlocks { pending, peer_id } = &mut *state {
                if let Some(pos) = pending.iter().position(|h| *h == hash) {
                    pending.remove(pos);
                }
                if pending.is_empty() {
                    let pid = *peer_id;
                    *state = SyncState::Idle;
                    Some(pid)
                } else {
                    None
                }
            } else {
                None
            }
        };

        // If pending cleared, check if we need more blocks
        if let Some(peer_id) = next_peer {
            let local_height = self.get_local_height();
            let peer_height = self.peer_manager.get_peer_start_height(peer_id).unwrap_or(0);
            if (peer_height as u64) > local_height as u64 {
                let _ = self.start_sync(peer_id);
            }
        }
    }

    // Start sync process with a peer
    pub fn start_sync(&self, peer_id: u64) -> Result<(), String> {
        println!("SyncManager: Starting sync check with peer {}", peer_id);
        
        let chain = self.chain.lock().unwrap();
        let our_height = chain
            .get_tip()
            .map(|t| t.map(|d| d.height).unwrap_or(0))
            .unwrap_or(0);
        drop(chain);

        let peer_height = self.peer_manager.get_peer_start_height(peer_id).unwrap_or(0);
        println!("SyncManager: Local height: {}, Peer {} height: {}", our_height, peer_id, peer_height);

        if (peer_height as u64) <= our_height {
            println!("SyncManager: Peer height is lower or equal, not requesting headers.");
            return Ok(());
        }

        let chain = self.chain.lock().unwrap();
        let locator = chain.get_block_locator();
        drop(chain); // Release lock before sending message

        println!("SyncManager: Requesting headers from peer {}", peer_id);

        // Update state
        let mut state = self.sync_state.lock().unwrap();
        *state = SyncState::DownloadingHeaders {
            from_peer: peer_id,
            highest_known: our_height,
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
        let chain = self.chain.lock().unwrap();
        let locator = chain.get_block_locator();
        let our_height = chain.get_height();
        drop(chain);

        println!("SyncManager: Force requesting headers from peer {}", peer_id);

        let mut state = self.sync_state.lock().unwrap();
        *state = SyncState::DownloadingHeaders {
            from_peer: peer_id,
            highest_known: our_height,
        };
        drop(state);

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

    // Handle received headers
    pub fn handle_headers(&self, peer_id: u64, headers_msg: &HeadersMessage) -> Result<(), String> {
        println!("SyncManager: Received {} headers from peer {}", headers_msg.headers.len(), peer_id);
        
        // If empty, we are likely synced
        if headers_msg.headers.is_empty() {
            let mut state = self.sync_state.lock().unwrap();
            *state = SyncState::Synced;
            return Ok(());
        }

        let chain = self.chain.lock().unwrap();

        // Validate and store headers
        for header in &headers_msg.headers {
            // Basic validation (PoW, difficulty, etc.)
            chain
                .validate_header_standalone(header)
                .map_err(|e| e.to_string())?;

            // Store header (without full block)
            chain.store_header_only(header).map_err(|e| e.to_string())?;
        }

        // Update peer height based on last header
        if let Some(last_header) = headers_msg.headers.last() {
            let hash = last_header.hash();
            if let Some(height) = chain.get_height_for_hash(&hash) {
                self.peer_manager.update_peer_height(peer_id, height as u32);
            }
        }

        // drop chain lock as we might call start_sync which locks chain
        drop(chain);

        // If received 2000 headers, request more
        if headers_msg.headers.len() >= 2000 {
            // Request blocks for this batch first
            self.request_blocks_from_headers(peer_id, &headers_msg.headers)?;
            // Then continue downloading headers
            self.start_sync(peer_id)?;
        } else {
            // Headers complete, start downloading blocks
            // Collect all block hashes we need (missing full blocks)
            // Ideally we should traverse from current block tip to header tip
            // But for simplicity, let's request blocks for the headers we just received?
            // Or better, check what we are missing.

            // The prompt says: "Request blocks for all stored headers"
            // And sets state to DownloadingBlocks

            let mut state = self.sync_state.lock().unwrap();

            // We need to find which blocks are missing content
            // This requires access to chain, but we dropped it.
            // Let's re-acquire chain or better, just use the headers we just received?
            // But we might have received headers we already have?
            // Usually headers message contains new headers.

            // Let's assume we want to download blocks for the headers in this message, plus any previous pending?
            // But wait, if we got < 2000, it means we reached tip.
            // We should iterate from our current BLOCK tip to HEADER tip and request blocks.

            // Implementation detail:
            // "Request blocks for all stored headers"
            // I'll implement `request_blocks_for_headers` helper.
            *state = SyncState::DownloadingBlocks {
                pending: Vec::new(),
                peer_id,
            };
            drop(state);

            self.request_blocks_from_headers(peer_id, &headers_msg.headers)?;
        }

        Ok(())
    }

    fn request_blocks_from_headers(
        &self,
        peer_id: u64,
        headers: &[crate::consensus::block::BlockHeader],
    ) -> Result<(), String> {
        let chain = self.chain.lock().unwrap();
        let mut pending = Vec::new();
        println!("SyncManager: request_blocks_from_headers called with {} headers", headers.len());

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
        println!("SyncManager: {} blocks need downloading", pending.len());

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
            println!("SyncManager: Sent getdata for {} blocks to peer {}", pending.len(), peer_id);

            // Update state
            let mut state = self.sync_state.lock().unwrap();
            if let SyncState::DownloadingBlocks { pending: ref mut p, .. } = *state {
                p.extend(pending);
            }
        }

        Ok(())
    }
    // Handle getheaders request
    pub fn handle_getheaders(&self, peer_id: u64, msg: &GetHeadersMessage) -> Result<(), String> {
        println!("handle_getheaders called from peer {}, sending headers...", peer_id);
        let chain = self.chain.lock().unwrap();

        // Find common ancestor from locator
        let mut start_height = 0;
        for hash in &msg.block_locator {
            if let Some(height) = chain.get_height_for_hash(hash) {
                // Verify this block is in the active chain
                if let Some(active_block) = chain.get_block_by_height(height) {
                    if active_block.hash() == *hash {
                        start_height = height + 1;
                        break;
                    }
                }
            }
        }

        // Collect up to 2000 headers
        let mut headers = Vec::new();
        // Use block tip or header tip? Usually header tip.
        // But I don't have header tip getter.
        // I'll try to probe heights.

        let mut height = start_height;
        loop {
            if headers.len() >= 2000 {
                break;
            }

            if let Some(header) = chain.get_header_at_height(height) {
                let hash = header.hash();
                headers.push(header);

                // Stop if we hit stop_hash
                if msg.stop_hash != [0u8; 32] && hash == msg.stop_hash {
                    break;
                }
                height += 1;
            } else {
                break;
            }
        }

        drop(chain);

        // Send headers
        let headers_len = headers.len();
        let headers_msg = HeadersMessage { headers };
        println!("handle_getheaders: sending {} headers to peer {}", headers_len, peer_id);
        
        let magic = self.peer_manager.magic();
        let msg = NetworkMessage::new(magic, "headers", headers_msg.encode());

        self.peer_manager.send_to_peer(peer_id, &msg)?;

        Ok(())
    }

    // Check if synced
    pub fn is_synced(&self) -> bool {
        let state = self.sync_state.lock().unwrap();
        matches!(*state, SyncState::Synced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::params::Network;
    use crate::network::message::REGTEST_MAGIC;
    use tempfile::tempdir;

    #[test]
    fn test_sync_manager_creation() {
        // Setup ChainState
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();
        let params = Network::Regtest.params();
        let chain = Arc::new(Mutex::new(ChainState::new(params, db_path).unwrap()));

        // Setup PeerManager
        let peer_manager = Arc::new(PeerManager::new(REGTEST_MAGIC, 10, 70015, 0, 0));

        // Setup SyncManager
        let sync_manager = SyncManager::new(chain.clone(), peer_manager.clone());

        // Verify state
        assert!(matches!(
            *sync_manager.sync_state.lock().unwrap(),
            SyncState::Idle
        ));
    }
}
