use crate::consensus::chain::ChainState;
use crate::consensus::transaction::{Transaction, TxKind, TxOutput};
use crate::consensus::utxo::OutPoint;
use crate::consensus::validation::validate_transaction_v2_mempool;
use crate::primitives::hash::Hash256;
use crate::script::engine::{validate_p2dl, ScriptContext};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

/// Maximum number of unconfirmed transactions held in memory.
/// Transactions beyond this limit are rejected; no eviction is performed.
pub const MEMPOOL_MAX_ENTRIES: usize = 5_000;

pub struct NetworkMempool {
    transactions: Arc<Mutex<HashMap<[u8; 32], TxKind>>>,
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
    pub fn add_transaction(&self, tx: TxKind) -> Result<bool, String> {
        let txid = tx.txid();

        // Validate inputs against chain state first (no mempool lock held here,
        // so the chain lock and mempool lock are never acquired simultaneously,
        // eliminating any lock-ordering deadlock risk).
        match &tx {
            TxKind::V1(v1) => self.validate_v1_against_chain(v1)?,
            TxKind::V2(v2) => {
                let chain = self.chain.read().unwrap();
                let tip_height = chain.get_height();
                validate_transaction_v2_mempool(v2, &chain, tip_height)
                    .map_err(|e| format!("V2 validation failed: {:?}", e))?;
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
        let new_inputs = tx.input_outpoints();
        for pending in mempool.values() {
            for pending_in in pending.input_outpoints() {
                if new_inputs.contains(&pending_in) {
                    return Err("Input already spent by a pending mempool transaction".to_string());
                }
            }
        }

        mempool.insert(txid, tx);
        Ok(true)
    }

    fn validate_v1_against_chain(&self, tx: &Transaction) -> Result<(), String> {
        let chain = self.chain.read().unwrap();
        let mut spent_outputs: Vec<TxOutput> = Vec::with_capacity(tx.inputs.len());
        for input in &tx.inputs {
            let outpoint = OutPoint {
                txid: input.prev_txid,
                vout: input.prev_index,
            };
            let utxo = chain
                .get_utxo(&outpoint)?
                .ok_or_else(|| "Input UTXO not found".to_string())?;
            spent_outputs.push(utxo.output);
        }
        for (index, input) in tx.inputs.iter().enumerate() {
            let script_pubkey = &spent_outputs[index].script_pubkey;
            let context = ScriptContext {
                transaction: tx,
                input_index: index,
                spent_outputs: &spent_outputs,
            };
            let is_p2dl = script_pubkey.len() == 36
                && script_pubkey[0] == 0xaa
                && script_pubkey[1] == 0x20
                && script_pubkey[34] == 0x88
                && script_pubkey[35] == 0xac;
            if is_p2dl {
                let ok = validate_p2dl(&input.script_sig, script_pubkey, &context)
                    .map_err(|e| format!("Script validation error: {:?}", e))?;
                if !ok {
                    return Err("P2DL script validation failed".to_string());
                }
            }
        }
        Ok(())
    }

    // Get transaction by hash
    pub fn get_transaction(&self, txid: &[u8; 32]) -> Option<TxKind> {
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
    pub fn remove_block_transactions(&self, block_txs: &[TxKind]) {
        // Collect all OutPoints spent by the confirmed block.
        let mut spent: HashSet<(Hash256, u32)> = HashSet::new();
        for tx in block_txs {
            for op in tx.input_outpoints() {
                spent.insert(op);
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
                .input_outpoints()
                .iter()
                .any(|op| spent.contains(op))
        });
    }

    // Get all transactions
    pub fn get_all_transactions(&self) -> Vec<TxKind> {
        let mempool = self.transactions.lock().unwrap();
        mempool.values().cloned().collect()
    }

    /// Outpoints (txid, vout) already spent by pending mempool transactions.
    /// Coin selection must exclude these to avoid building conflicting double-spends.
    pub fn spent_outpoints(&self) -> HashSet<(Hash256, u32)> {
        let mempool = self.transactions.lock().unwrap();
        mempool
            .values()
            .flat_map(|tx| tx.input_outpoints())
            .collect()
    }

    // Clear mempool
    pub fn clear(&self) {
        let mut mempool = self.transactions.lock().unwrap();
        mempool.clear();
    }

    /// Evict any mempool transaction whose inputs are no longer in the UTXO set.
    /// Must be called with no chain lock held — acquires chain.read() internally.
    /// This is a no-op when there has been no reorg; on reorg it removes transactions
    /// whose inputs were on the disconnected chain and no longer exist.
    pub fn purge_stale(&self) {
        let chain = self.chain.read().unwrap();
        let mut mempool = self.transactions.lock().unwrap();
        let before = mempool.len();
        mempool.retain(|_, tx| match tx {
            TxKind::V1(v1) => v1.inputs.iter().all(|input| {
                let outpoint = OutPoint {
                    txid: input.prev_txid,
                    vout: input.prev_index,
                };
                chain.is_utxo_unspent(&outpoint)
            }),
            TxKind::V2(v2) => v2.inputs.iter().all(|input| {
                let outpoint = OutPoint {
                    txid: input.prev_txid,
                    vout: input.prev_index,
                };
                chain
                    .get_utxo_v2(&outpoint)
                    .map(|o| o.is_some())
                    .unwrap_or(false)
                    || chain.is_utxo_unspent(&outpoint)
            }),
        });
        let evicted = before - mempool.len();
        if evicted > 0 {
            log::info!(
                "Mempool: purged {} stale transaction(s) after reorg",
                evicted
            );
        }
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
        let result = mempool.add_transaction(TxKind::V1(tx.clone()));
        assert!(result.is_ok());
        assert_eq!(mempool.size(), 1);
        assert!(mempool.has_transaction(&tx.txid()));

        // Insert again
        let result = mempool.add_transaction(TxKind::V1(tx.clone()));
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Returns false (already exists)

        // Remove
        mempool.remove_transaction(&tx.txid());
        assert_eq!(mempool.size(), 0);
    }
}
