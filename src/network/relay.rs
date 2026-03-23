use crate::consensus::chain::{ChainError, ChainState};
use crate::consensus::transaction::Transaction;
use crate::network::manager::PeerManager;
use crate::network::mempool::NetworkMempool;
use crate::network::message::{NetworkMessage, CMD_BLOCK, CMD_GETDATA, CMD_INV, CMD_TX};
use crate::network::protocol::{
    BlockMessage, GetDataMessage, InvMessage, InvVector, TxMessage, INV_BLOCK, INV_TX,
};
use crate::primitives::serialize::Encode;
use std::collections::HashSet;

const MAX_ANNOUNCED_BLOCKS: usize = 1000;
use std::sync::{Arc, Mutex, RwLock};

pub struct BlockRelay {
    chain: Arc<RwLock<ChainState>>,
    peer_manager: Arc<PeerManager>,
    announced_blocks: Arc<Mutex<HashSet<[u8; 32]>>>,
    mempool: Arc<NetworkMempool>,
}

impl BlockRelay {
    pub fn new(
        chain: Arc<RwLock<ChainState>>,
        peer_manager: Arc<PeerManager>,
        mempool: Arc<NetworkMempool>,
    ) -> Self {
        Self {
            chain,
            peer_manager,
            announced_blocks: Arc::new(Mutex::new(HashSet::new())),
            mempool,
        }
    }

    // Announce new block to all peers
    pub fn announce_block(&self, block_hash: [u8; 32]) -> Result<(), String> {
        // Check if already announced
        let mut announced = self.announced_blocks.lock().unwrap();
        if announced.contains(&block_hash) {
            return Ok(()); // Already announced
        }

        // Prune when full — old entries are no longer useful (peers already have those blocks).
        // Worst case: we re-announce a pruned hash, which the peer silently ignores.
        if announced.len() >= MAX_ANNOUNCED_BLOCKS {
            announced.clear();
        }

        // Mark as announced
        announced.insert(block_hash);

        // Create INV message
        let inv = InvMessage {
            inventory: vec![InvVector {
                inv_type: INV_BLOCK,
                hash: block_hash,
            }],
        };

        let magic = self.peer_manager.magic();
        let msg = NetworkMessage::new(magic, CMD_INV, inv.encode());

        // Broadcast to all active peers
        self.peer_manager.broadcast(&msg)?;

        Ok(())
    }

    // Handle received INV message
    pub fn handle_inv(&self, peer_id: u64, inv: &InvMessage) -> Result<(), String> {
        let mut trigger_sync = false;
        let mut to_request = Vec::new();

        {
            let chain = self.chain.read().unwrap();
            for inv_vec in &inv.inventory {
                match inv_vec.inv_type {
                    INV_BLOCK => {
                        if !chain.has_block(&inv_vec.hash) {
                            trigger_sync = true;
                        }
                    }
                    INV_TX => {
                        if !self.mempool.has_transaction(&inv_vec.hash) {
                            to_request.push(*inv_vec);
                        }
                    }
                    _ => {}
                }
            }
        }

        if trigger_sync {
            let sync_guard = self.peer_manager.sync_manager();
            let sync = sync_guard.lock().unwrap();
            if let Some(sync) = &*sync {
                if !sync.is_syncing() {
                    let _ = sync.request_headers_force(peer_id);
                }
            }
        }

        if !to_request.is_empty() {
            let getdata = GetDataMessage {
                inventory: to_request,
            };
            let magic = self.peer_manager.magic();
            let msg = NetworkMessage::new(magic, CMD_GETDATA, getdata.encode());
            self.peer_manager.send_to_peer(peer_id, &msg)?;
        }

        Ok(())
    }

    // Handle received GetData message
    pub fn handle_getdata(&self, peer_id: u64, getdata: &GetDataMessage) -> Result<(), String> {
        let magic = self.peer_manager.magic();
        self.handle_getdata_inner(peer_id, getdata, magic)
    }

    fn handle_getdata_inner(
        &self,
        peer_id: u64,
        getdata: &GetDataMessage,
        magic: [u8; 4],
    ) -> Result<(), String> {
        // Handle Blocks
        let mut blocks_to_send = Vec::new();
        let mut txs_to_send = Vec::new();
        log::debug!(
            "Relay: handle_getdata called from peer {}, {} items",
            peer_id,
            getdata.inventory.len()
        );

        {
            let chain = self.chain.read().unwrap();
            for inv_vec in &getdata.inventory {
                match inv_vec.inv_type {
                    INV_BLOCK => {
                        let block_opt = chain
                            .get_block(&inv_vec.hash)
                            .map(|b| BlockMessage {
                                header: b.header,
                                transactions: b.transactions.clone(),
                            })
                            .or_else(|| {
                                chain
                                    .block_store
                                    .get_block(&inv_vec.hash)
                                    .ok()
                                    .flatten()
                                    .map(|b| BlockMessage {
                                        header: b.header,
                                        transactions: b.transactions,
                                    })
                            });
                        if let Some(block_msg) = block_opt {
                            log::debug!(
                                "Relay: serving block {} to peer {}",
                                hex::encode(inv_vec.hash),
                                peer_id
                            );
                            blocks_to_send.push(block_msg);
                        } else {
                            log::debug!(
                                "Relay: block {} not found for peer {}",
                                hex::encode(inv_vec.hash),
                                peer_id
                            );
                        }
                    }
                    INV_TX => {
                        if let Some(tx) = self.mempool.get_transaction(&inv_vec.hash) {
                            txs_to_send.push(TxMessage { transaction: tx });
                        }
                    }
                    _ => {}
                }
            }
        }

        for block_msg in blocks_to_send {
            let msg = NetworkMessage::new(magic, CMD_BLOCK, block_msg.encode());
            self.peer_manager.send_to_peer(peer_id, &msg)?;
        }

        for tx_msg in txs_to_send {
            let msg = NetworkMessage::new(magic, CMD_TX, tx_msg.encode());
            self.peer_manager.send_to_peer(peer_id, &msg)?;
        }

        Ok(())
    }

    // Handle received Block message
    pub fn handle_block(&self, peer_id: u64, block: &BlockMessage) -> Result<(), String> {
        let block_hash = block.header.hash();
        log::debug!(
            "Relay: received block {} from peer {}",
            hex::encode(block_hash),
            peer_id
        );

        // Try routing through SyncManager first.  During a fork-sync the manager
        // buffers out-of-order blocks and applies them sequentially, preventing
        // OrphanBlock errors caused by gaps in the fork chain.
        use crate::consensus::block::Block;
        let maybe_applied = {
            let sync_guard = self.peer_manager.sync_manager();
            let sync_lock = sync_guard.lock().unwrap();
            sync_lock.as_ref().and_then(|sync| {
                sync.receive_block_for_sync(Block {
                    header: block.header,
                    transactions: block.transactions.clone(),
                })
            })
        };

        if let Some((applied_pairs, requeued_txs)) = maybe_applied {
            // SyncManager handled this block (applied in order or buffered).
            // applied_pairs is empty when the block was only buffered.
            // Each entry is (block, height) for blocks actually committed.
            for (applied, height) in &applied_pairs {
                log::info!(
                    "Relay: sync applied block {} at height {}",
                    hex::encode(applied.header.hash()),
                    height
                );
                self.mempool
                    .remove_block_transactions(&applied.transactions);
            }
            if !applied_pairs.is_empty() {
                self.mempool.purge_stale();
                // Re-add transactions from disconnected blocks so they can be re-mined.
                for tx in requeued_txs {
                    let _ = self.mempool.add_transaction(tx);
                }
            }
            // Update peer height from the last applied block.  The height comes directly
            // from the sync session's peer_header_map so it's valid for fork-chain blocks
            // that may not yet be on the canonical chain.
            if let Some((_, height)) = applied_pairs.last() {
                self.peer_manager.update_peer_height(peer_id, *height);
            }
            return Ok(());
        }

        // Normal relay path: not a tracked sync block.
        let mut chain = self.chain.write().unwrap();

        if chain.has_block(&block_hash) {
            let height = chain.get_height_for_hash(&block_hash).or_else(|| {
                chain
                    .block_store
                    .get_block_meta(&block_hash)
                    .ok()
                    .flatten()
                    .map(|m| m.height)
            });
            drop(chain);
            if let Some(h) = height {
                self.peer_manager.update_peer_height(peer_id, h as u32);
            }
            return Ok(());
        }

        match chain.add_block(Block {
            header: block.header,
            transactions: block.transactions.clone(),
        }) {
            Ok(requeued_txs) => {
                log::info!(
                    "Relay: block {} added to chain successfully",
                    hex::encode(block_hash)
                );

                let height = chain.get_height_for_hash(&block_hash);

                let is_tip = chain
                    .get_tip()
                    .map(|t| {
                        t.map(|d| d.block.header.hash() == block_hash)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);

                drop(chain);

                if let Some(h) = height {
                    self.peer_manager.update_peer_height(peer_id, h as u32);
                }

                if is_tip {
                    let syncing = {
                        let sync_guard = self.peer_manager.sync_manager();
                        let sync = sync_guard.lock().unwrap();
                        sync.as_ref().map(|s| s.is_syncing()).unwrap_or(false)
                    };

                    if !syncing {
                        self.announce_block(block_hash)?;
                    }
                    self.mempool.remove_block_transactions(&block.transactions);
                    self.mempool.purge_stale();
                    // Re-add transactions from disconnected blocks so they can be re-mined.
                    for tx in requeued_txs {
                        let _ = self.mempool.add_transaction(tx);
                    }
                }
                Ok(())
            }
            Err(ChainError::OrphanBlock) => {
                log::debug!(
                    "Relay: block {} is orphan (parent={})",
                    hex::encode(block_hash),
                    hex::encode(block.header.prev_block_hash)
                );
                drop(chain);
                let sync_guard = self.peer_manager.sync_manager();
                let sync = sync_guard.lock().unwrap();
                if let Some(sync) = &*sync {
                    // Store the orphan so it can be applied once its parent arrives.
                    sync.store_orphan(
                        block_hash,
                        crate::consensus::block::Block {
                            header: block.header,
                            transactions: block.transactions.clone(),
                        },
                    );
                    if !sync.is_syncing() {
                        let _ = sync.request_headers_force(peer_id);
                    }
                }
                Ok(())
            }
            Err(e) => {
                log::warn!(
                    "Relay: block {} validation failed: {:?}",
                    hex::encode(block_hash),
                    e
                );
                Err(format!("Block validation failed: {}", e))
            }
        }
    }

    // Announce new transaction
    pub fn announce_transaction(&self, txid: [u8; 32]) -> Result<(), String> {
        let inv = InvMessage {
            inventory: vec![InvVector {
                inv_type: INV_TX,
                hash: txid,
            }],
        };

        let magic = self.peer_manager.magic();
        let msg = NetworkMessage::new(magic, CMD_INV, inv.encode());

        self.peer_manager.broadcast(&msg)?;
        Ok(())
    }

    // Handle received transaction
    pub fn handle_transaction(&self, _peer_id: u64, tx: &Transaction) -> Result<(), String> {
        match self.mempool.add_transaction(tx.clone()) {
            Ok(true) => {
                let txid = tx.txid();
                // Relay to other peers
                self.announce_transaction(txid)?;
                Ok(())
            }
            Ok(false) => Ok(()), // Already have it
            Err(e) => Err(format!("Transaction rejected: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::params::Network;
    use crate::network::message::REGTEST_MAGIC;
    use tempfile::tempdir;

    #[test]
    fn test_block_relay_creation() {
        // Setup ChainState
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();
        let params = Network::Regtest.params();
        let chain = Arc::new(RwLock::new(ChainState::new(params, db_path).unwrap()));

        // Setup PeerManager
        let peer_manager = Arc::new(PeerManager::new(REGTEST_MAGIC, 10, 70015, 0, 0));

        // Setup Mempool
        let mempool = Arc::new(NetworkMempool::new(chain.clone()));

        // Setup BlockRelay
        let relay = Arc::new(BlockRelay::new(chain, peer_manager.clone(), mempool));

        // Link them
        peer_manager.set_relay(relay.clone());

        // Test announce_block (should succeed even with 0 peers)
        let block_hash = [0u8; 32];
        assert!(relay.announce_block(block_hash).is_ok());

        // Verify it's in announced set
        // We can't access private field announced_blocks directly from test module unless we expose it or use pub(crate)
        // But we can check if calling announce again returns Ok
        // Actually announce_block returns Ok(()) if already announced.
        // We can't easily verify side effects without peers.
    }
}
