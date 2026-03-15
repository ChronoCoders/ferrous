use crate::consensus::chain::ChainState;
use crate::consensus::transaction::Transaction;
use crate::consensus::utxo::OutPoint;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Maximum number of unconfirmed transactions held in memory.
/// Transactions beyond this limit are rejected; no eviction is performed.
pub const MEMPOOL_MAX_ENTRIES: usize = 5_000;

pub struct NetworkMempool {
    transactions: Arc<Mutex<HashMap<[u8; 32], Transaction>>>,
    chain: Arc<Mutex<ChainState>>,
}

impl NetworkMempool {
    pub fn new(chain: Arc<Mutex<ChainState>>) -> Self {
        Self {
            transactions: Arc::new(Mutex::new(HashMap::new())),
            chain,
        }
    }

    // Add transaction to mempool
    pub fn add_transaction(&self, tx: Transaction) -> Result<bool, String> {
        let txid = tx.txid();

        // Validate inputs against chain state first (no mempool lock held here,
        // so the chain lock and mempool lock are never acquired simultaneously,
        // eliminating any lock-ordering deadlock risk).
        {
            let chain = self.chain.lock().unwrap();
            for input in &tx.inputs {
                let outpoint = OutPoint {
                    txid: input.prev_txid,
                    vout: input.prev_index,
                };
                if !chain.is_utxo_unspent(&outpoint) {
                    return Err("Input already spent or doesn't exist".to_string());
                }
            }
        }

        // Duplicate check, size cap, and insert are all performed under a single
        // mempool lock, making the entire check-then-act sequence atomic and
        // eliminating the TOCTOU race that existed when two separate lock scopes
        // were used.
        let mut mempool = self.transactions.lock().unwrap();

        if mempool.contains_key(&txid) {
            return Ok(false); // Already have it
        }

        if mempool.len() >= MEMPOOL_MAX_ENTRIES {
            return Err(format!(
                "Mempool full ({} entries): transaction rejected",
                MEMPOOL_MAX_ENTRIES
            ));
        }

        mempool.insert(txid, tx);
        Ok(true)
    }

    // Get transaction by hash
    pub fn get_transaction(&self, txid: &[u8; 32]) -> Option<Transaction> {
        let mempool = self.transactions.lock().unwrap();
        mempool.get(txid).cloned()
    }

    // Check if transaction exists
    pub fn has_transaction(&self, txid: &[u8; 32]) -> bool {
        let mempool = self.transactions.lock().unwrap();
        mempool.contains_key(txid)
    }

    // Remove transaction (after it's mined)
    pub fn remove_transaction(&self, txid: &[u8; 32]) {
        let mut mempool = self.transactions.lock().unwrap();
        mempool.remove(txid);
    }

    // Remove transactions that are in a block
    pub fn remove_block_transactions(&self, block_txs: &[Transaction]) {
        let mut mempool = self.transactions.lock().unwrap();
        for tx in block_txs {
            mempool.remove(&tx.txid());
        }
    }

    // Get all transactions
    pub fn get_all_transactions(&self) -> Vec<Transaction> {
        let mempool = self.transactions.lock().unwrap();
        mempool.values().cloned().collect()
    }

    // Clear mempool
    pub fn clear(&self) {
        let mut mempool = self.transactions.lock().unwrap();
        mempool.clear();
    }

    // Get mempool size
    pub fn size(&self) -> usize {
        let mempool = self.transactions.lock().unwrap();
        mempool.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::params::Network;
    use tempfile::tempdir;

    #[test]
    fn test_mempool_lifecycle() {
        // Setup ChainState
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();
        let params = Network::Regtest.params();
        let chain = Arc::new(Mutex::new(ChainState::new(params, db_path).unwrap()));

        let mempool = NetworkMempool::new(chain.clone());

        assert_eq!(mempool.size(), 0);

        // We can't easily create a valid transaction without valid UTXOs in chain
        // But we can test basic insertion if validation passes or fails correctly.

        // Create dummy tx
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![],
            witnesses: vec![],
            locktime: 0,
        };

        // Should fail because inputs empty? Or pass if empty allowed?
        // Logic only checks inputs if they exist.
        // If inputs empty, it passes "is_utxo_unspent" check (loop doesn't run).

        // Insert
        let result = mempool.add_transaction(tx.clone());
        assert!(result.is_ok());
        assert_eq!(mempool.size(), 1);
        assert!(mempool.has_transaction(&tx.txid()));

        // Insert again
        let result = mempool.add_transaction(tx.clone());
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Returns false (already exists)

        // Remove
        mempool.remove_transaction(&tx.txid());
        assert_eq!(mempool.size(), 0);
    }
}
