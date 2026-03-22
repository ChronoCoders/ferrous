use crate::consensus::chain::ChainState;
use crate::consensus::transaction::Transaction;
use crate::consensus::utxo::OutPoint;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// Maximum number of unconfirmed transactions held in memory.
/// Transactions beyond this limit are rejected; no eviction is performed.
pub const MEMPOOL_MAX_ENTRIES: usize = 5_000;

pub struct NetworkMempool {
    transactions: Arc<Mutex<HashMap<[u8; 32], Transaction>>>,
    chain: Arc<RwLock<ChainState>>,
}

impl NetworkMempool {
    pub fn new(chain: Arc<RwLock<ChainState>>) -> Self {
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
            let chain = self.chain.read().unwrap();
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

        // Duplicate check, conflict check, size cap, and insert are all performed
        // under a single mempool lock, making the entire check-then-act sequence
        // atomic and eliminating the TOCTOU race that existed when two separate
        // lock scopes were used.
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

        // Reject if any input conflicts with an already-pending mempool transaction.
        // Without this check, two sendtoaddress calls within the same block interval
        // both pass the chain UTXO check (the UTXO is still unspent on-chain) and
        // produce two transactions spending the same UTXO. build_template then
        // includes both, and add_block fails on the second with UtxoNotFound.
        for pending in mempool.values() {
            for pending_in in &pending.inputs {
                for new_in in &tx.inputs {
                    if new_in.prev_txid == pending_in.prev_txid
                        && new_in.prev_index == pending_in.prev_index
                    {
                        return Err(
                            "Input already spent by a pending mempool transaction".to_string()
                        );
                    }
                }
            }
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

    // Remove transactions that are in a block, plus any mempool transactions
    // that conflict with block transactions (spend the same inputs). This handles
    // the case where a peer mines a block containing a transaction we didn't have
    // in our mempool, but which spends the same UTXO as one we do have.
    pub fn remove_block_transactions(&self, block_txs: &[Transaction]) {
        // Collect all OutPoints spent by the confirmed block.
        let mut spent: std::collections::HashSet<([u8; 32], u32)> =
            std::collections::HashSet::new();
        for tx in block_txs {
            for input in &tx.inputs {
                spent.insert((input.prev_txid, input.prev_index));
            }
        }

        let mut mempool = self.transactions.lock().unwrap();
        // Remove exact matches first.
        for tx in block_txs {
            mempool.remove(&tx.txid());
        }
        // Evict any remaining mempool tx that spends a now-confirmed input.
        mempool.retain(|_, pending| {
            !pending
                .inputs
                .iter()
                .any(|i| spent.contains(&(i.prev_txid, i.prev_index)))
        });
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
        let chain = Arc::new(RwLock::new(ChainState::new(params, db_path).unwrap()));

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
