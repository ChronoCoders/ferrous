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
use std::sync::{Arc, Mutex};

pub struct BlockRelay {
    chain: Arc<Mutex<ChainState>>,
    peer_manager: Arc<PeerManager>,
    announced_blocks: Arc<Mutex<HashSet<[u8; 32]>>>,
    mempool: Arc<NetworkMempool>,
}

impl BlockRelay {
    pub fn new(
        chain: Arc<Mutex<ChainState>>,
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
        let chain = self.chain.lock().unwrap();
        let mut to_request = Vec::new();

        for inv_vec in &inv.inventory {
            match inv_vec.inv_type {
                INV_BLOCK => {
                    if !chain.has_block(&inv_vec.hash) {
                        // Trigger headers sync instead of direct GETDATA
                        // This avoids INV rate limit issues during bulk mining
                        drop(chain);
                        let sync_guard = self.peer_manager.sync_manager();
                        let sync = sync_guard.lock().unwrap();
                        if let Some(sync) = &*sync {
                            let _ = sync.request_headers_force(peer_id);
                        }
                        return Ok(());
                    }
                }
                INV_TX => {
                    // Check if we have this tx
                    if !self.mempool.has_transaction(&inv_vec.hash) {
                        // Also check chain? Usually unnecessary if we assume confirmed txs are not relayed via INV.
                        // But good practice to check if already mined?
                        // For now just check mempool.
                        to_request.push(*inv_vec);
                    }
                }
                _ => {}
            }
        }

        drop(chain);

        // Request unknown items
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

        {
            let chain = self.chain.lock().unwrap();
            for inv_vec in &getdata.inventory {
                match inv_vec.inv_type {
                    INV_BLOCK => {
                        if let Some(block) = chain.get_block(&inv_vec.hash) {
                            blocks_to_send.push(BlockMessage {
                                header: block.header,
                                transactions: block.transactions.clone(),
                            });
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

        // Notify SyncManager that we received this block (to clear pending state)
        if let Some(sync) = &*self.peer_manager.sync_manager().lock().unwrap() {
            sync.block_received(block_hash);
        }

        let mut chain = self.chain.lock().unwrap();

        // Validate and add block
        use crate::consensus::block::Block;
        match chain.add_block(Block {
            header: block.header,
            transactions: block.transactions.clone(),
        }) {
            Ok(()) => {
                let height = chain.get_height_for_hash(&block_hash);

                // Check if it's the new tip
                // Note: get_tip returns Option<BlockData>. We unwrap result then Option.
                let is_tip = chain
                    .get_tip()
                    .map(|t| {
                        t.map(|d| d.block.header.hash() == block_hash)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);

                drop(chain);

                // Update peer height
                if let Some(h) = height {
                    self.peer_manager.update_peer_height(peer_id, h as u32);
                }

                if is_tip {
                    // Check if syncing
                    let syncing = {
                        let sync_guard = self.peer_manager.sync_manager();
                        let sync = sync_guard.lock().unwrap();
                        sync.as_ref().map(|s| s.is_syncing()).unwrap_or(false)
                    };

                    if !syncing {
                        self.announce_block(block_hash)?;
                    }
                    // Remove mined transactions from mempool
                    self.mempool.remove_block_transactions(&block.transactions);
                }
                Ok(())
            }
            Err(ChainError::OrphanBlock) => {
                drop(chain);
                // Trigger header sync to find parent
                let sync_guard = self.peer_manager.sync_manager();
                let sync = sync_guard.lock().unwrap();
                if let Some(sync) = &*sync {
                    let _ = sync.request_headers_force(peer_id);
                }
                Ok(())
            }
            Err(e) => Err(format!("Block validation failed: {}", e)),
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
        let chain = Arc::new(Mutex::new(ChainState::new(params, db_path).unwrap()));

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
