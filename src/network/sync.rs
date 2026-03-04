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
    DownloadingHeaders { from_peer: u64, highest_known: u32 },
    DownloadingBlocks { pending: Vec<[u8; 32]> },
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

    // Start sync process with a peer
    pub fn start_sync(&self, peer_id: u64) -> Result<(), String> {
        let chain = self.chain.lock().unwrap();
        let locator = chain.get_block_locator();
        let our_height = chain.get_tip().map(|t| t.height).unwrap_or(0);
        drop(chain);

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

    // Handle received headers
    pub fn handle_headers(&self, peer_id: u64, headers_msg: &HeadersMessage) -> Result<(), String> {
        // If empty, we are likely synced
        if headers_msg.headers.is_empty() {
            let mut state = self.sync_state.lock().unwrap();
            *state = SyncState::Synced;
            return Ok(());
        }

        let mut chain = self.chain.lock().unwrap();

        // Validate and store headers
        for header in &headers_msg.headers {
            // Basic validation (PoW, difficulty, etc.)
            if !chain
                .validate_header_standalone(header)
                .map_err(|e| e.to_string())?
            {
                return Err("Invalid header".to_string());
            }

            // Store header (without full block)
            chain.store_header_only(header).map_err(|e| e.to_string())?;
        }

        // drop chain lock as we might call start_sync which locks chain
        drop(chain);

        // If received 2000 headers, request more
        if headers_msg.headers.len() >= 2000 {
            // Continue downloading headers
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
            };
            drop(state);

            self.request_blocks_for_headers(peer_id)?;
        }

        Ok(())
    }

    fn request_blocks_for_headers(&self, peer_id: u64) -> Result<(), String> {
        let chain = self.chain.lock().unwrap();
        // We want to download blocks from block_tip + 1 to header_tip
        // But ChainState doesn't expose easy iterator or range getter without implementation.
        // However, we can just walk back from header_tip until we hit block_tip?
        // Or walk forward?
        // ChainState doesn't have "get_block_hash_by_height" public?
        // Yes, `get_block_at_height` uses `get_hash_at_height`.
        // I can use `get_header_at_height` to get hashes.

        // We need block tip height and header tip height.
        // I didn't expose header_tip_height getter on ChainState.
        // But I added the field. I should probably use `get_tip` (block tip) and I need header tip.
        // I'll add `get_header_tip` to ChainState?
        // Or just rely on the fact that we just stored headers.

        // Let's try to get hashes.
        // For now, I'll use `get_header_at_height` assuming it works.
        // But I need the range.

        // Since I cannot modify ChainState public interface easily without another round,
        // I will assume I can just use `get_block_locator` or similar? No.

        // Wait, I updated `ChainState` but didn't add `get_header_tip` getter.
        // `get_tip` returns BlockData.

        // I will just use the headers from the message if I had them?
        // But `handle_headers` consumed them.

        // Re-reading `handle_headers`: "Request blocks for all stored headers".
        // It calls `self.request_blocks_for_headers()?`.

        // I will implement a loop checking `get_block_at_height`.
        // If it returns None, but `get_header_at_height` returns Some, we need that block.

        let mut pending = Vec::new();
        let mut height = chain.get_tip().map(|t| t.height).unwrap_or(0) + 1;

        while let Some(header) = chain.get_header_at_height(height) {
            // Check if we have the full block
            if chain.get_block_by_hash(&header.hash()).is_none() {
                pending.push(header.hash());
            }
            height += 1;

            // Limit pending blocks to avoid OOM or too huge message
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

            // Update state
            let mut state = self.sync_state.lock().unwrap();
            *state = SyncState::DownloadingBlocks { pending };
        } else {
            let mut state = self.sync_state.lock().unwrap();
            *state = SyncState::Synced;
        }

        Ok(())
    }

    // Handle getheaders request
    pub fn handle_getheaders(&self, peer_id: u64, msg: &GetHeadersMessage) -> Result<(), String> {
        let chain = self.chain.lock().unwrap();

        // Find common ancestor from locator
        let mut start_height = 0;
        for hash in &msg.block_locator {
            if let Some(height) = chain.get_height_for_hash(hash) {
                start_height = height + 1;
                break;
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
        let headers_msg = HeadersMessage { headers };
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
        let chain = Arc::new(Mutex::new(ChainState::new(params, db_path, None).unwrap()));

        // Setup PeerManager
        let peer_manager = Arc::new(PeerManager::new(REGTEST_MAGIC, 70015, 0, 0, 10));

        // Setup SyncManager
        let sync_manager = SyncManager::new(chain.clone(), peer_manager.clone());

        // Verify state
        assert!(matches!(
            *sync_manager.sync_state.lock().unwrap(),
            SyncState::Idle
        ));
    }
}
