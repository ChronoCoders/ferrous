use crate::consensus::block::{Block, BlockData, BlockHeader, U256};
use crate::consensus::difficulty::{validate_difficulty, DifficultyError};
use crate::consensus::params::ChainParams;
use crate::consensus::utxo::{OutPoint, UtxoEntry, UtxoError};
use crate::consensus::validation::{
    validate_block, validate_coinbase_height, validate_coinbase_reward, validate_timestamp,
    ValidationError, COINBASE_MATURITY,
};
use crate::primitives::hash::Hash256;
use crate::script::engine::{validate_p2pkh, ScriptContext};
use crate::storage::{BlockStore, ChainStateStore, ChainTip, Database, UtxoStore};
use log::info;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainError {
    BlockNotFound,
    InvalidBlock(ValidationError),
    InvalidDifficulty(DifficultyError),
    UtxoError(UtxoError),
    OrphanBlock,
    DbError(String),
    GenesisMismatch,
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainError::BlockNotFound => write!(f, "Block not found"),
            ChainError::InvalidBlock(e) => write!(f, "Invalid block: {:?}", e),
            ChainError::InvalidDifficulty(e) => write!(f, "Invalid difficulty: {:?}", e),
            ChainError::UtxoError(e) => write!(f, "UTXO error: {:?}", e),
            ChainError::OrphanBlock => write!(f, "Orphan block"),
            ChainError::DbError(e) => write!(f, "Database error: {}", e),
            ChainError::GenesisMismatch => write!(f, "Genesis block mismatch"),
        }
    }
}

impl std::error::Error for ChainError {}

impl From<String> for ChainError {
    fn from(e: String) -> Self {
        ChainError::DbError(e)
    }
}

pub struct ChainState {
    pub params: ChainParams,

    // Storage backends
    pub db: Arc<Database>,
    pub block_store: Arc<BlockStore>,
    pub utxo_store: Arc<UtxoStore>,
    pub state_store: Arc<ChainStateStore>,

    // In-memory index (block hash -> BlockData)
    blocks: HashMap<Hash256, BlockData>,

    // Cached tip (synced with storage)
    tip: Option<Hash256>,
}

type ApplyBlockToUtxoResult = (
    Vec<(OutPoint, UtxoEntry)>,
    Vec<OutPoint>,
    Vec<(OutPoint, UtxoEntry)>,
    u64,
);

impl ChainState {
    /// Create new chain state with persistent storage
    pub fn new<P: AsRef<Path>>(params: ChainParams, db_path: P) -> Result<Self, String> {
        // Open database
        let db = Arc::new(Database::open(db_path)?);

        // Create stores
        let block_store = Arc::new(BlockStore::new(Arc::clone(&db)));
        let utxo_store = Arc::new(UtxoStore::new(Arc::clone(&db)));
        let state_store = Arc::new(ChainStateStore::new(Arc::clone(&db)));

        let mut chain = Self {
            params,
            db,
            block_store,
            utxo_store,
            state_store,
            blocks: HashMap::new(),
            tip: None,
        };

        // Recover from storage if initialized
        chain.recover_from_storage()?;

        Ok(chain)
    }

    /// Create in-memory chain state (for testing)
    pub fn new_in_memory(params: ChainParams) -> Result<Self, String> {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().map_err(|e: std::io::Error| e.to_string())?;
        let db_path = temp_dir.path().to_path_buf();

        let chain = Self::new(params, &db_path)?;

        // Keep temp_dir alive by leaking it (acceptable for tests)
        std::mem::forget(temp_dir);

        Ok(chain)
    }

    /// Recover chain state from storage
    fn recover_from_storage(&mut self) -> Result<(), String> {
        // Check if chain is initialized
        if !self.state_store.is_initialized()? {
            info!("Chain not initialized, starting fresh");
            return Ok(());
        }

        // Load tip
        let chain_tip = self
            .state_store
            .get_tip()?
            .ok_or("Chain initialized but no tip found")?;

        info!(
            "Recovering chain state: height={}, hash={}",
            chain_tip.height,
            hex::encode(chain_tip.hash)
        );

        // Load blocks into memory index
        // Start from genesis and load all blocks up to tip
        // This is inefficient for large chains but acceptable for now.
        // A better approach would be to load headers only or load on demand.
        // But the prompt says "In-memory index (block hash -> BlockData)"
        // For 100 blocks it's instant.

        // We need to reconstruct the chain from tip backwards to genesis, then load them.
        let mut current_hash = chain_tip.hash;
        let mut chain_hashes = Vec::new();

        // Loop backwards
        loop {
            chain_hashes.push(current_hash);
            let header = self
                .block_store
                .get_header(&current_hash)?
                .ok_or_else(|| format!("Missing header for {}", hex::encode(current_hash)))?;

            if header.prev_block_hash == [0u8; 32] {
                break;
            }
            current_hash = header.prev_block_hash;
        }

        // Timestamp and PoW re-validation is intentionally skipped here.
        // Every block was validated in full (including validate_timestamp) when it
        // was first accepted via add_block().  Re-validating on startup would be
        // expensive and would incorrectly reject historical headers whose timestamps
        // are now "far in the future" relative to the network-adjusted wall clock.
        // A --reindex flag could be added in future to force full re-validation.

        // Forward pass: load blocks from genesis toward tip, validating canonical
        // chain linkage at each step.
        //
        // Two invariants are enforced per block:
        //   1. The block's computed hash matches the key recorded during backward
        //      traversal (detects a block stored under the wrong key).
        //   2. The block's prev_block_hash equals the hash of the immediately
        //      preceding block in the canonical sequence (detects any gap or fork
        //      in the stored chain).
        let mut expected_prev_hash: Option<Hash256> = None;

        for hash in chain_hashes.iter().rev() {
            let block = self
                .block_store
                .get_block(hash)?
                .ok_or_else(|| format!("Missing block {}", hex::encode(hash)))?;

            let header = block.header;
            let block_hash = header.hash();

            // Invariant 1: stored key must match the block's actual hash.
            if block_hash != *hash {
                return Err(format!(
                    "Recovery: hash mismatch — stored key {} but block hashes to {}",
                    hex::encode(hash),
                    hex::encode(block_hash)
                ));
            }

            // Invariant 2: chain must be strictly linear.
            match expected_prev_hash {
                None => {
                    // First block processed must be genesis (prev == all-zeros).
                    if header.prev_block_hash != [0u8; 32] {
                        return Err(format!(
                            "Recovery: first block {} is not genesis (prev_hash={})",
                            hex::encode(block_hash),
                            hex::encode(header.prev_block_hash)
                        ));
                    }
                }
                Some(prev) => {
                    if header.prev_block_hash != prev {
                        return Err(format!(
                            "Recovery: chain linkage broken at block {} \
                             (expected prev={}, got {})",
                            hex::encode(block_hash),
                            hex::encode(prev),
                            hex::encode(header.prev_block_hash)
                        ));
                    }
                }
            }

            expected_prev_hash = Some(block_hash);

            let (height, cumulative_work) = if header.prev_block_hash == [0u8; 32] {
                (0, header.work())
            } else {
                let prev_data = self
                    .blocks
                    .get(&header.prev_block_hash)
                    .ok_or("Previous block not loaded during recovery")?;
                (
                    prev_data.height + 1,
                    prev_data.cumulative_work + header.work(),
                )
            };

            self.blocks.insert(
                block_hash,
                BlockData {
                    block,
                    height,
                    cumulative_work,
                },
            );

            // Repair CF_BLOCK_INDEX for this canonical height.  An incomplete reorg or
            // a side-chain store_block call may have left a stale hash here; overwriting
            // it on every startup guarantees CF_BLOCK_INDEX matches self.blocks exactly.
            self.block_store
                .update_height_index(height, &block_hash)
                .map_err(|e| {
                    format!(
                        "Recovery: failed to repair height index at {}: {}",
                        height, e
                    )
                })?;
        }

        self.tip = Some(chain_tip.hash);

        info!("Recovery complete: {} blocks loaded", self.blocks.len());
        Ok(())
    }

    fn reorganize(&mut self, old_tip: &Hash256, new_tip: &Hash256) -> Result<(), ChainError> {
        info!(
            "Reorganizing chain from {} to {}",
            hex::encode(old_tip),
            hex::encode(new_tip)
        );

        let mut old_chain = Vec::new();
        let mut new_chain = Vec::new();

        let mut old_curr = *old_tip;
        let mut new_curr = *new_tip;

        let old_height = self.blocks.get(&old_curr).map(|d| d.height).unwrap_or(0);
        let new_height = self.blocks.get(&new_curr).map(|d| d.height).unwrap_or(0);

        while self.blocks.get(&old_curr).map(|d| d.height).unwrap_or(0) > new_height {
            old_chain.push(old_curr);
            if let Some(data) = self.blocks.get(&old_curr) {
                old_curr = data.block.header.prev_block_hash;
            } else {
                break;
            }
        }

        while self.blocks.get(&new_curr).map(|d| d.height).unwrap_or(0) > old_height {
            new_chain.push(new_curr);
            if let Some(data) = self.blocks.get(&new_curr) {
                new_curr = data.block.header.prev_block_hash;
            } else {
                break;
            }
        }

        while old_curr != new_curr {
            if old_curr == [0u8; 32] || new_curr == [0u8; 32] {
                return Err(ChainError::DbError(format!(
                    "Reorg failed: no common ancestor found between {} and {}",
                    hex::encode(old_tip),
                    hex::encode(new_tip)
                )));
            }
            old_chain.push(old_curr);
            new_chain.push(new_curr);
            if let Some(data) = self.blocks.get(&old_curr) {
                old_curr = data.block.header.prev_block_hash;
            } else {
                return Err(ChainError::DbError(format!(
                    "Reorg failed: block {} not found in memory index",
                    hex::encode(old_curr)
                )));
            }
            if let Some(data) = self.blocks.get(&new_curr) {
                new_curr = data.block.header.prev_block_hash;
            } else {
                return Err(ChainError::DbError(format!(
                    "Reorg failed: block {} not found in memory index",
                    hex::encode(new_curr)
                )));
            }
        }

        info!(
            "Reorg: disconnecting {} blocks, reconnecting {}",
            old_chain.len(),
            new_chain.len()
        );

        for block_hash in &old_chain {
            let block = self
                .blocks
                .get(block_hash)
                .ok_or(ChainError::BlockNotFound)?
                .block
                .clone();

            let mut created_outpoints = Vec::new();
            for tx in &block.transactions {
                let txid = tx.txid();
                for (vout, _) in tx.outputs.iter().enumerate() {
                    created_outpoints.push(OutPoint {
                        txid,
                        vout: vout as u32,
                    });
                }
            }

            let restored_entries = self
                .utxo_store
                .get_undo_data(block_hash)
                .map_err(ChainError::DbError)?
                .ok_or_else(|| {
                    ChainError::DbError(format!(
                        "Missing undo data for block {}",
                        hex::encode(block_hash)
                    ))
                })?;

            self.utxo_store
                .revert_block(&created_outpoints, &restored_entries)
                .map_err(ChainError::DbError)?;

            self.utxo_store
                .delete_undo_data(block_hash)
                .map_err(ChainError::DbError)?;
        }

        for hash in new_chain.iter().rev() {
            if let Some(data) = self.blocks.get(hash) {
                let block = data.block.clone();
                let height = data.height;
                let bh = block.hash();
                let (created_utxos, spent_utxos, spent_entries, fees) =
                    self.apply_block_to_utxo(&block, height)?;

                validate_coinbase_reward(&block.transactions[0], fees, height as u32)
                    .map_err(ChainError::InvalidBlock)?;

                self.utxo_store
                    .apply_block(&created_utxos, &spent_utxos)
                    .map_err(ChainError::DbError)?;
                self.utxo_store
                    .store_undo_data(&bh, &spent_entries)
                    .map_err(ChainError::DbError)?;

                // Reconnected blocks become part of the canonical chain: update
                // the height index so height-based lookups return the correct hash.
                // (These blocks were originally stored via store_block_no_index when
                // they arrived as side-chain candidates.)
                self.block_store
                    .update_height_index(height, hash)
                    .map_err(ChainError::DbError)?;
            }
        }

        Ok(())
    }

    pub fn add_block(&mut self, block: Block) -> Result<(), ChainError> {
        let block_hash = block.hash();

        validate_block(&block.header, &block.transactions).map_err(ChainError::InvalidBlock)?;

        let (height, cumulative_work) = if block.header.prev_block_hash == [0u8; 32] {
            (0, block.header.work())
        } else {
            // Check memory first, then DB
            let prev_hash_hex = hex::encode(block.header.prev_block_hash);
            println!(
                "add_block: looking for parent {} in memory: {}",
                prev_hash_hex,
                self.blocks.contains_key(&block.header.prev_block_hash)
            );
            if !self.blocks.contains_key(&block.header.prev_block_hash) {
                let db_result = self.block_store.get_block(&block.header.prev_block_hash);
                println!(
                    "add_block: DB lookup for parent {}: {:?}",
                    prev_hash_hex,
                    db_result.as_ref().map(|r| r.is_some())
                );
                if let Ok(Some(prev_block)) = db_result {
                    let prev_hash = prev_block.header.hash();
                    let meta = self
                        .block_store
                        .get_block_meta(&prev_hash)
                        .map_err(ChainError::DbError)?
                        .ok_or_else(|| ChainError::DbError("Missing block meta".to_string()))?;
                    let prev_height = meta.height;
                    let prev_data_owned = BlockData {
                        block: prev_block,
                        height: prev_height,
                        cumulative_work: meta.cumulative_work,
                    };
                    self.blocks.insert(prev_hash, prev_data_owned);
                }
            }

            let prev_data = self
                .blocks
                .get(&block.header.prev_block_hash)
                .ok_or(ChainError::OrphanBlock)?;

            validate_difficulty(Some(&prev_data.block.header), &block.header, &self.params)
                .map_err(ChainError::InvalidDifficulty)?;

            let mut prev_headers = Vec::new();
            let mut curr = block.header.prev_block_hash;
            for _ in 0..11 {
                if let Some(d) = self.blocks.get(&curr) {
                    prev_headers.push(d.block.header);
                    if d.block.header.prev_block_hash == [0u8; 32] {
                        break;
                    }
                    curr = d.block.header.prev_block_hash;
                } else {
                    break;
                }
            }
            validate_timestamp(&block.header, &prev_headers).map_err(ChainError::InvalidBlock)?;

            (
                prev_data.height + 1,
                prev_data.cumulative_work + block.header.work(),
            )
        };

        if height > 0 {
            validate_coinbase_height(&block.transactions[0], height as u32)
                .map_err(ChainError::InvalidBlock)?;
        }

        self.blocks.insert(
            block_hash,
            BlockData {
                block: block.clone(),
                height,
                cumulative_work,
            },
        );

        let should_update_tip = match self.tip {
            None => true,
            Some(current_tip) => {
                let current_work = self
                    .blocks
                    .get(&current_tip)
                    .map(|d| d.cumulative_work)
                    .unwrap_or(U256::from(0u64));
                cumulative_work > current_work
            }
        };

        if should_update_tip {
            let did_reorg = if let Some(current_tip_hash) = self.tip {
                if block.header.prev_block_hash != current_tip_hash {
                    self.reorganize(&current_tip_hash, &block_hash)?;
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if !did_reorg {
                let (created_utxos, spent_utxos, spent_entries, block_fees) =
                    self.apply_block_to_utxo(&block, height)?;

                validate_coinbase_reward(&block.transactions[0], block_fees, height as u32)
                    .map_err(ChainError::InvalidBlock)?;

                self.utxo_store
                    .apply_block(&created_utxos, &spent_utxos)
                    .map_err(ChainError::DbError)?;
                self.utxo_store
                    .store_undo_data(&block_hash, &spent_entries)
                    .map_err(ChainError::DbError)?;
            }

            self.block_store
                .store_block(&block, height, cumulative_work)
                .map_err(ChainError::DbError)?;

            let new_tip = ChainTip {
                hash: block_hash,
                height,
                cumulative_work,
            };
            self.state_store
                .set_tip(&new_tip)
                .map_err(ChainError::DbError)?;
            self.tip = Some(block_hash);
        } else {
            // Side-chain block: persist data but do NOT update the canonical
            // height index (CF_BLOCK_INDEX).  Updating it here would overwrite
            // the active chain's height → hash mapping and corrupt any lookup
            // that relies on it (e.g. get_block_by_height).
            self.block_store
                .store_block_no_index(&block, height, cumulative_work)
                .map_err(ChainError::DbError)?;
        }

        Ok(())
    }

    fn apply_block_to_utxo(
        &self,
        block: &Block,
        height: u64,
    ) -> Result<ApplyBlockToUtxoResult, ChainError> {
        let mut created = Vec::new();
        let mut spent = Vec::new();
        let mut spent_entries = Vec::new();
        let mut block_fees: u64 = 0;

        for (tx_idx, tx) in block.transactions.iter().enumerate() {
            let txid = tx.txid();
            let is_coinbase = tx_idx == 0;

            if !is_coinbase {
                let mut spent_outputs = Vec::with_capacity(tx.inputs.len());
                let mut input_sum: u64 = 0;
                let mut seen_outpoints: HashSet<OutPoint> = HashSet::with_capacity(tx.inputs.len());

                for input in &tx.inputs {
                    let outpoint = OutPoint {
                        txid: input.prev_txid,
                        vout: input.prev_index,
                    };

                    if !seen_outpoints.insert(outpoint) {
                        return Err(ChainError::UtxoError(UtxoError::UtxoAlreadySpent));
                    }

                    let utxo_entry = self
                        .utxo_store
                        .get_utxo(&outpoint)
                        .map_err(ChainError::DbError)?
                        .ok_or(ChainError::UtxoError(UtxoError::UtxoNotFound))?;

                    if utxo_entry.coinbase
                        && height < utxo_entry.height.saturating_add(COINBASE_MATURITY)
                    {
                        return Err(ChainError::UtxoError(UtxoError::ImmatureCoinbase));
                    }

                    input_sum = input_sum
                        .checked_add(utxo_entry.output.value)
                        .ok_or(ChainError::UtxoError(UtxoError::ValueOverflow))?;

                    spent.push(outpoint);
                    spent_entries.push((outpoint, utxo_entry.clone()));
                    spent_outputs.push(utxo_entry.output);
                }

                let output_sum: u64 = tx.outputs.iter().map(|o| o.value).sum();
                if input_sum < output_sum {
                    return Err(ChainError::UtxoError(UtxoError::InsufficientValue));
                }

                let fee = input_sum
                    .checked_sub(output_sum)
                    .ok_or(ChainError::UtxoError(UtxoError::InsufficientValue))?;
                block_fees = block_fees
                    .checked_add(fee)
                    .ok_or(ChainError::UtxoError(UtxoError::ValueOverflow))?;

                for (input_idx, input) in tx.inputs.iter().enumerate() {
                    let script_pubkey = &spent_outputs[input_idx].script_pubkey;
                    let context = ScriptContext {
                        transaction: tx,
                        input_index: input_idx,
                        spent_outputs: &spent_outputs,
                    };

                    match validate_p2pkh(&input.script_sig, script_pubkey, &context) {
                        Ok(true) => {}
                        _ => return Err(ChainError::UtxoError(UtxoError::ScriptValidationFailed)),
                    }
                }
            }

            for (vout, output) in tx.outputs.iter().enumerate() {
                let outpoint = OutPoint {
                    txid,
                    vout: vout as u32,
                };
                let entry = UtxoEntry {
                    output: output.clone(),
                    coinbase: is_coinbase,
                    height,
                };
                created.push((outpoint, entry));
            }
        }

        Ok((created, spent, spent_entries, block_fees))
    }

    pub fn get_utxo(&self, outpoint: &OutPoint) -> Result<Option<UtxoEntry>, String> {
        self.utxo_store.get_utxo(outpoint)
    }

    pub fn get_block(&self, hash: &Hash256) -> Option<&Block> {
        self.blocks.get(hash).map(|data| &data.block)
    }

    pub fn get_block_by_height(&self, height: u64) -> Option<&Block> {
        // Use storage for lookup, then get from memory
        if let Ok(Some(block)) = self.block_store.get_block_by_height(height) {
            let hash = block.hash();
            return self.blocks.get(&hash).map(|data| &data.block);
        }
        None
    }

    pub fn get_tip(&self) -> Result<Option<BlockData>, String> {
        match self.tip {
            Some(hash) => {
                let data = self.blocks.get(&hash).cloned();
                Ok(data)
            }
            None => Ok(None),
        }
    }

    pub fn get_height(&self) -> u64 {
        match self.get_tip() {
            Ok(Some(tip)) => tip.height,
            _ => 0,
        }
    }

    pub fn export_utxos(&self) -> Result<Vec<(OutPoint, UtxoEntry)>, ChainError> {
        self.utxo_store.export_utxos().map_err(ChainError::DbError)
    }

    pub fn median_time_past(&self) -> u64 {
        let tip = match self.tip {
            Some(h) => h,
            None => return 0,
        };

        let mut timestamps = Vec::new();
        let mut current = tip;

        for _ in 0..11 {
            if let Some(data) = self.blocks.get(&current) {
                timestamps.push(data.block.header.timestamp);
                if data.block.header.prev_block_hash == [0u8; 32] {
                    break;
                }
                current = data.block.header.prev_block_hash;
            } else {
                break;
            }
        }

        if timestamps.is_empty() {
            return 0;
        }

        timestamps.sort_unstable();
        let len = timestamps.len();
        // For a full chain len == 11 and the median is index 5.
        // Near genesis (len < 11) we use all available ancestors; the median
        // index is still len/2, which is correct for both odd and even counts.
        timestamps[len / 2]
    }

    pub fn is_utxo_unspent(&self, outpoint: &OutPoint) -> bool {
        self.utxo_store.has_utxo(outpoint).unwrap_or(false)
    }

    pub fn has_block(&self, hash: &Hash256) -> bool {
        self.block_store.has_block(hash).unwrap_or(false)
    }

    pub fn get_block_hash(&self, height: u64) -> Option<Hash256> {
        let mut current_hash = self.tip?;
        let mut current_data = self.blocks.get(&current_hash)?;

        if height > current_data.height {
            return None;
        }

        // Walk back
        while current_data.height > height {
            current_hash = current_data.block.header.prev_block_hash;
            current_data = self.blocks.get(&current_hash)?;
        }

        Some(current_hash)
    }

    pub fn get_block_locator(&self) -> Vec<Hash256> {
        self.get_block_locator_with_heights()
            .into_iter()
            .map(|(hash, _)| hash)
            .collect()
    }

    pub fn get_block_locator_with_heights(&self) -> Vec<(Hash256, u64)> {
        let mut entries = Vec::new();

        let tip = match self.state_store.get_tip() {
            Ok(Some(tip)) => tip,
            _ => return vec![[0u8; 32]].into_iter().map(|h| (h, 0)).collect(),
        };

        entries.push((tip.hash, tip.height));

        let mut step: u64 = 1;
        let mut current_height = tip.height;

        while current_height > 0 {
            if entries.len() >= 10 {
                step = step.saturating_mul(2);
            }
            current_height = current_height.saturating_sub(step);

            // Use CF_BLOCK_INDEX (canonical chain) — NOT get_header_hash_by_height
            // which reads CF_HEADERS first.  CF_HEADERS is overwritten by
            // store_headers_batch on every sync session, so after syncing from a
            // divergent peer the height keys there point to that peer's hashes,
            // not our canonical chain.  Sending a locator full of the peer's own
            // hashes causes the peer to match at the wrong height and send only
            // a tail of its chain instead of headers from the true fork point.
            match self.block_store.get_hash_by_height(current_height) {
                Ok(Some(hash)) => entries.push((hash, current_height)),
                _ => break,
            }
        }

        entries
    }

    pub fn get_header_at_height(&self, height: u64) -> Option<BlockHeader> {
        self.block_store.get_header_by_height(height).ok().flatten()
    }

    pub fn get_height_for_hash(&self, hash: &Hash256) -> Option<u64> {
        self.blocks.get(hash).map(|d| d.height)
    }

    pub fn validate_header_standalone(&self, _header: &BlockHeader) -> Result<(), String> {
        Ok(())
    }

    pub fn store_header_only(&self, header: &BlockHeader) -> Result<(), String> {
        self.block_store
            .store_header(header)
            .map_err(|e| e.to_string())
    }

    pub fn store_header_only_at_height(
        &self,
        header: &BlockHeader,
        height: u64,
    ) -> Result<(), String> {
        self.block_store
            .store_header_at_height(header, height)
            .map_err(|e| e.to_string())
    }
}

// Add impl Add for U256 if not exists, or wrapper
use std::ops::Add;
impl Add for U256 {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        // Simple 256-bit addition with carry
        let mut result = [0u8; 32];
        let mut carry = 0u16;

        for (i, (a, b)) in self.0.iter().zip(other.0.iter()).enumerate() {
            let sum = (*a as u16) + (*b as u16) + carry;
            result[i] = (sum & 0xFF) as u8;
            carry = sum >> 8;
        }
        // Ignore overflow for work accumulation (unlikely to reach 2^256 work)
        U256(result)
    }
}

// Implement Ord/PartialOrd for U256 if not already in block.rs
// It IS in block.rs.
