use crate::consensus::block::{Block, BlockData, BlockHeader, U256};
use crate::consensus::difficulty::{validate_difficulty, DifficultyError};
use crate::consensus::params::ChainParams;
use crate::consensus::utxo::{OutPoint, UtxoEntry, UtxoError};
use crate::consensus::validation::{
    validate_block, validate_coinbase_height, validate_coinbase_reward, validate_timestamp,
    ValidationError, COINBASE_MATURITY,
};
use crate::primitives::hash::Hash256;
use crate::script::engine::{validate_p2dl, ScriptContext};
use crate::storage::{
    BlockStore, ChainStateStore, ChainTip, Database, UtxoStore, CF_UNDO, CF_UTXO,
};
use log::info;
use lru::LruCache;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
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

    // In-memory LRU cache (block hash -> BlockData); keeps the most recently
    // accessed blocks to bound memory usage.  Older blocks are served from RocksDB.
    blocks: LruCache<Hash256, BlockData>,

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

        const BLOCK_CACHE_SIZE: usize = 2048;
        let mut chain = Self {
            params,
            db,
            block_store,
            utxo_store,
            state_store,
            blocks: LruCache::new(NonZeroUsize::new(BLOCK_CACHE_SIZE).unwrap()),
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

        // Load the most recent BLOCK_CACHE_SIZE blocks into the LRU cache.
        // Older blocks remain in RocksDB and are fetched on demand.
        let cache_capacity = self.blocks.cap().get();

        let mut current_hash = chain_tip.hash;
        let mut chain_hashes = Vec::new();

        // Walk backwards from tip, collecting up to cache_capacity hashes.
        loop {
            chain_hashes.push(current_hash);
            if chain_hashes.len() >= cache_capacity {
                break;
            }
            let header = self
                .block_store
                .get_header(&current_hash)?
                .ok_or_else(|| format!("Missing header for {}", hex::encode(current_hash)))?;

            if header.prev_block_hash == [0u8; 32] {
                break;
            }
            current_hash = header.prev_block_hash;
        }

        // Forward pass: load blocks oldest-first into the LRU cache.
        // Height and cumulative_work come from stored BlockMeta (set at accept time),
        // so no incremental recomputation is needed.  We still verify hash integrity
        // and chain linkage within the loaded window.
        //
        // Two invariants are enforced per block:
        //   1. The block's computed hash matches the key recorded during backward
        //      traversal (detects a block stored under the wrong key).
        //   2. The block's prev_block_hash equals the hash of the immediately
        //      preceding block in the canonical sequence (detects any gap or fork
        //      in the stored chain within the loaded window).
        let mut expected_prev_hash: Option<Hash256> = None;

        for hash in chain_hashes.iter().rev() {
            let block = self
                .block_store
                .get_block(hash)?
                .ok_or_else(|| format!("Missing block {}", hex::encode(hash)))?;

            let meta = self
                .block_store
                .get_block_meta(hash)?
                .ok_or_else(|| format!("Missing block meta for {}", hex::encode(hash)))?;

            let block_hash = block.header.hash();

            // Invariant 1: stored key must match the block's actual hash.
            if block_hash != *hash {
                return Err(format!(
                    "Recovery: hash mismatch — stored key {} but block hashes to {}",
                    hex::encode(hash),
                    hex::encode(block_hash)
                ));
            }

            // Invariant 2: chain linkage within the loaded window.
            if let Some(prev) = expected_prev_hash {
                if block.header.prev_block_hash != prev {
                    return Err(format!(
                        "Recovery: chain linkage broken at block {} \
                         (expected prev={}, got {})",
                        hex::encode(block_hash),
                        hex::encode(prev),
                        hex::encode(block.header.prev_block_hash)
                    ));
                }
            }

            expected_prev_hash = Some(block_hash);

            let height = meta.height;
            let cumulative_work = meta.cumulative_work;

            self.blocks.put(
                block_hash,
                BlockData {
                    block,
                    height,
                    cumulative_work,
                },
            );

            // Repair CF_BLOCK_INDEX for this canonical height.
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

    // Compact 36-byte key for a UTXO outpoint (must match UtxoStore's encoding).
    fn utxo_key(op: &OutPoint) -> Vec<u8> {
        let mut key = Vec::with_capacity(36);
        key.extend_from_slice(&op.txid);
        key.extend_from_slice(&op.vout.to_le_bytes());
        key
    }

    // Serialize a UtxoEntry (must match UtxoStore's encoding).
    fn utxo_val(entry: &UtxoEntry) -> Result<Vec<u8>, ChainError> {
        bincode::serialize(entry)
            .map_err(|e| ChainError::DbError(format!("UTXO serialize failed: {}", e)))
    }

    fn reorganize(
        &mut self,
        old_tip: &Hash256,
        new_tip: &Hash256,
    ) -> Result<Vec<crate::consensus::transaction::Transaction>, ChainError> {
        info!(
            "Reorganizing chain from {} to {}",
            hex::encode(old_tip),
            hex::encode(new_tip)
        );

        let mut old_chain = Vec::new();
        let mut new_chain = Vec::new();

        let mut old_curr = *old_tip;
        let mut new_curr = *new_tip;

        // Helper: get block height from cache (peek) or DB.
        let block_height = |blocks: &LruCache<Hash256, BlockData>,
                            block_store: &BlockStore,
                            hash: &Hash256|
         -> u64 {
            if let Some(d) = blocks.peek(hash) {
                return d.height;
            }
            block_store
                .get_block_meta(hash)
                .ok()
                .flatten()
                .map(|m| m.height)
                .unwrap_or(0)
        };

        // Helper: get prev_block_hash from cache (peek) or DB.
        let prev_hash_of = |blocks: &LruCache<Hash256, BlockData>,
                            block_store: &BlockStore,
                            hash: &Hash256|
         -> Option<Hash256> {
            if let Some(d) = blocks.peek(hash) {
                return Some(d.block.header.prev_block_hash);
            }
            block_store
                .get_header(hash)
                .ok()
                .flatten()
                .map(|h| h.prev_block_hash)
        };

        let old_height = block_height(&self.blocks, &self.block_store, &old_curr);
        let new_height = block_height(&self.blocks, &self.block_store, &new_curr);

        while block_height(&self.blocks, &self.block_store, &old_curr) > new_height {
            old_chain.push(old_curr);
            match prev_hash_of(&self.blocks, &self.block_store, &old_curr) {
                Some(prev) => old_curr = prev,
                None => break,
            }
        }

        while block_height(&self.blocks, &self.block_store, &new_curr) > old_height {
            new_chain.push(new_curr);
            match prev_hash_of(&self.blocks, &self.block_store, &new_curr) {
                Some(prev) => new_curr = prev,
                None => break,
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
            match prev_hash_of(&self.blocks, &self.block_store, &old_curr) {
                Some(prev) => old_curr = prev,
                None => {
                    return Err(ChainError::DbError(format!(
                        "Reorg failed: block {} not found in cache or DB",
                        hex::encode(old_curr)
                    )))
                }
            }
            match prev_hash_of(&self.blocks, &self.block_store, &new_curr) {
                Some(prev) => new_curr = prev,
                None => {
                    return Err(ChainError::DbError(format!(
                        "Reorg failed: block {} not found in cache or DB",
                        hex::encode(new_curr)
                    )))
                }
            }
        }

        info!(
            "Reorg: disconnecting {} blocks, reconnecting {}",
            old_chain.len(),
            new_chain.len()
        );

        // ── Phase 1: collect disconnect data; build UTXO overlay ─────────────
        //
        // The overlay tracks the post-disconnect UTXO state in memory so that
        // reconnect validation can see the correct view without touching the DB.
        // None  = outpoint deleted (was created by an old block being disconnected)
        // Some  = outpoint exists (was restored from undo data)
        let mut overlay: HashMap<OutPoint, Option<UtxoEntry>> = HashMap::new();

        struct DisconnectEntry {
            block_hash: Hash256,
            block: Block,
            undo_data: Vec<(OutPoint, UtxoEntry)>,
            created_outpoints: Vec<OutPoint>,
        }

        let mut disconnect_entries: Vec<DisconnectEntry> = Vec::with_capacity(old_chain.len());

        for block_hash in &old_chain {
            let block = if let Some(d) = self.blocks.peek(block_hash) {
                d.block.clone()
            } else {
                self.block_store
                    .get_block(block_hash)
                    .map_err(ChainError::DbError)?
                    .ok_or(ChainError::BlockNotFound)?
            };

            let undo_data = self
                .utxo_store
                .get_undo_data(block_hash)
                .map_err(ChainError::DbError)?
                .ok_or_else(|| {
                    ChainError::DbError(format!(
                        "Missing undo data for block {}",
                        hex::encode(block_hash)
                    ))
                })?;

            // Remove UTXOs created by this old block from the overlay view.
            let mut created_outpoints = Vec::new();
            for tx in &block.transactions {
                let txid = tx.txid();
                for (vout, _) in tx.outputs.iter().enumerate() {
                    let op = OutPoint {
                        txid,
                        vout: vout as u32,
                    };
                    overlay.insert(op, None);
                    created_outpoints.push(op);
                }
            }
            // Restore UTXOs that were spent by this old block.
            for (op, entry) in &undo_data {
                overlay.insert(*op, Some(entry.clone()));
            }

            disconnect_entries.push(DisconnectEntry {
                block_hash: *block_hash,
                block,
                undo_data,
                created_outpoints,
            });
        }

        // ── Phase 2: validate reconnect blocks against the overlay (no DB writes) ──

        struct ReconnectEntry {
            block_hash: Hash256,
            height: u64,
            created: Vec<(OutPoint, UtxoEntry)>,
            spent: Vec<OutPoint>,
            undo_data: Vec<(OutPoint, UtxoEntry)>,
        }

        let mut reconnect_entries: Vec<ReconnectEntry> = Vec::with_capacity(new_chain.len());

        for hash in new_chain.iter().rev() {
            let (block, height) = if let Some(d) = self.blocks.peek(hash) {
                (d.block.clone(), d.height)
            } else {
                let b = self
                    .block_store
                    .get_block(hash)
                    .map_err(ChainError::DbError)?
                    .ok_or(ChainError::BlockNotFound)?;
                let meta = self
                    .block_store
                    .get_block_meta(hash)
                    .map_err(ChainError::DbError)?
                    .ok_or_else(|| {
                        ChainError::DbError(format!("Missing block meta for {}", hex::encode(hash)))
                    })?;
                (b, meta.height)
            };

            let (created, spent, undo, fees) =
                self.apply_block_to_utxo_sim(&block, height, &overlay)?;

            validate_coinbase_reward(&block.transactions[0], fees, height as u32)
                .map_err(ChainError::InvalidBlock)?;

            // Advance the overlay with this block's changes.
            for op in &spent {
                overlay.insert(*op, None);
            }
            for (op, entry) in &created {
                overlay.insert(*op, Some(entry.clone()));
            }

            reconnect_entries.push(ReconnectEntry {
                block_hash: *hash,
                height,
                created,
                spent,
                undo_data: undo,
            });
        }

        // ── Phase 3: all validation passed — commit all changes atomically ───

        let mut batch = self.db.batch();

        for entry in &disconnect_entries {
            // Delete UTXOs that were created by each old block.
            for op in &entry.created_outpoints {
                batch
                    .delete(CF_UTXO, &Self::utxo_key(op))
                    .map_err(ChainError::DbError)?;
            }
            // Restore UTXOs that were spent by each old block.
            for (op, utxo_entry) in &entry.undo_data {
                batch
                    .put(CF_UTXO, &Self::utxo_key(op), &Self::utxo_val(utxo_entry)?)
                    .map_err(ChainError::DbError)?;
            }
            // Remove the undo record for this old block.
            batch
                .delete(CF_UNDO, &entry.block_hash)
                .map_err(ChainError::DbError)?;
        }

        for entry in &reconnect_entries {
            // Remove UTXOs spent by each new block.
            for op in &entry.spent {
                batch
                    .delete(CF_UTXO, &Self::utxo_key(op))
                    .map_err(ChainError::DbError)?;
            }
            // Create UTXOs produced by each new block.
            for (op, utxo_entry) in &entry.created {
                batch
                    .put(CF_UTXO, &Self::utxo_key(op), &Self::utxo_val(utxo_entry)?)
                    .map_err(ChainError::DbError)?;
            }
            // Store the undo record for each new block.
            let undo_bytes = bincode::serialize(&entry.undo_data)
                .map_err(|e| ChainError::DbError(format!("undo serialize: {}", e)))?;
            batch
                .put(CF_UNDO, &entry.block_hash, &undo_bytes)
                .map_err(ChainError::DbError)?;
        }

        batch.commit().map_err(ChainError::DbError)?;

        // Update the canonical height index after the batch commits.
        // This is idempotent and can be repaired by recover_from_storage on restart.
        for entry in &reconnect_entries {
            self.block_store
                .update_height_index(entry.height, &entry.block_hash)
                .map_err(ChainError::DbError)?;
        }

        // Collect non-coinbase transactions from disconnected blocks whose
        // inputs are still unspent on the new chain.
        let mut requeued: Vec<crate::consensus::transaction::Transaction> = Vec::new();
        for entry in &disconnect_entries {
            for (tx_idx, tx) in entry.block.transactions.iter().enumerate() {
                if tx_idx == 0 {
                    continue;
                }
                let all_valid = tx.inputs.iter().all(|input| {
                    let op = OutPoint {
                        txid: input.prev_txid,
                        vout: input.prev_index,
                    };
                    match overlay.get(&op) {
                        Some(Some(_)) => true,
                        Some(None) => false,
                        None => self.utxo_store.has_utxo(&op).unwrap_or(false),
                    }
                });
                if all_valid {
                    requeued.push(tx.clone());
                }
            }
        }

        Ok(requeued)
    }

    pub fn add_block(
        &mut self,
        block: Block,
    ) -> Result<Vec<crate::consensus::transaction::Transaction>, ChainError> {
        let block_hash = block.hash();

        // Skip only if the block is already stored in the DB (canonical or side-chain).
        // A block that is in the LRU cache but NOT in block_store was put there
        // during a previous add_block call that later failed (e.g. a failed reorg).
        // That block must still reach the should_update_tip comparison so the reorg
        // can be retried.
        if self
            .block_store
            .has_block(&block_hash)
            .map_err(ChainError::DbError)?
        {
            return Ok(Vec::new());
        }

        let (height, cumulative_work) = if block.header.prev_block_hash == [0u8; 32] {
            (0, block.header.work())
        } else {
            // Check cache first, then DB
            if self.blocks.peek(&block.header.prev_block_hash).is_none() {
                if let Ok(Some(prev_block)) =
                    self.block_store.get_block(&block.header.prev_block_hash)
                {
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
                    self.blocks.put(prev_hash, prev_data_owned);
                }
            }

            let prev_data = self
                .blocks
                .peek(&block.header.prev_block_hash)
                .ok_or(ChainError::OrphanBlock)?;

            validate_difficulty(Some(&prev_data.block.header), &block.header, &self.params)
                .map_err(ChainError::InvalidDifficulty)?;

            let mut prev_headers = Vec::new();
            let mut curr = block.header.prev_block_hash;
            for _ in 0..11 {
                if let Some(d) = self.blocks.peek(&curr) {
                    prev_headers.push(d.block.header);
                    if d.block.header.prev_block_hash == [0u8; 32] {
                        break;
                    }
                    curr = d.block.header.prev_block_hash;
                    continue;
                }

                if let Ok(Some(hdr)) = self.block_store.get_header(&curr) {
                    prev_headers.push(hdr);
                    if hdr.prev_block_hash == [0u8; 32] {
                        break;
                    }
                    curr = hdr.prev_block_hash;
                    continue;
                }

                break;
            }
            validate_timestamp(&block.header, &prev_headers).map_err(ChainError::InvalidBlock)?;

            (
                prev_data.height + 1,
                prev_data.cumulative_work + block.header.work(),
            )
        };

        validate_block(&block.header, &block.transactions, height as u32)
            .map_err(ChainError::InvalidBlock)?;

        if height > 0 {
            validate_coinbase_height(&block.transactions[0], height as u32)
                .map_err(ChainError::InvalidBlock)?;
        }

        self.blocks.put(
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
                    .peek(&current_tip)
                    .map(|d| d.cumulative_work)
                    .unwrap_or(U256::from(0u64));
                cumulative_work > current_work
                    || (cumulative_work == current_work && block_hash < current_tip)
            }
        };

        if should_update_tip {
            let (did_reorg, requeued_txs) = if let Some(current_tip_hash) = self.tip {
                if block.header.prev_block_hash != current_tip_hash {
                    let requeued = self.reorganize(&current_tip_hash, &block_hash)?;
                    (true, requeued)
                } else {
                    (false, Vec::new())
                }
            } else {
                (false, Vec::new())
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
            return Ok(requeued_txs);
        } else {
            // Side-chain block: persist data but do NOT update the canonical
            // height index (CF_BLOCK_INDEX).  Updating it here would overwrite
            // the active chain's height → hash mapping and corrupt any lookup
            // that relies on it (e.g. get_block_by_height).
            self.block_store
                .store_block_no_index(&block, height, cumulative_work)
                .map_err(ChainError::DbError)?;
        }

        Ok(Vec::new())
    }

    // Core UTXO application logic, parameterised over the UTXO lookup source.
    // Used by both the normal (DB-backed) and the reorg-simulation (overlay-backed) paths.
    fn apply_block_to_utxo_inner<F>(
        block: &Block,
        height: u64,
        mut get_utxo: F,
    ) -> Result<ApplyBlockToUtxoResult, ChainError>
    where
        F: FnMut(&OutPoint) -> Result<Option<UtxoEntry>, ChainError>,
    {
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

                    let utxo_entry = get_utxo(&outpoint)?
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

                    match validate_p2dl(&input.script_sig, script_pubkey, &context) {
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

    fn apply_block_to_utxo(
        &self,
        block: &Block,
        height: u64,
    ) -> Result<ApplyBlockToUtxoResult, ChainError> {
        let utxo = &self.utxo_store;
        Self::apply_block_to_utxo_inner(block, height, |op| {
            utxo.get_utxo(op).map_err(ChainError::DbError)
        })
    }

    // Validates a block's UTXO changes against an in-memory overlay that reflects
    // the post-disconnect state, without writing anything to the DB.  Used by
    // the atomic reorg to validate all reconnect blocks before committing.
    fn apply_block_to_utxo_sim(
        &self,
        block: &Block,
        height: u64,
        overlay: &HashMap<OutPoint, Option<UtxoEntry>>,
    ) -> Result<ApplyBlockToUtxoResult, ChainError> {
        let utxo = &self.utxo_store;
        Self::apply_block_to_utxo_inner(block, height, |op| match overlay.get(op) {
            Some(Some(entry)) => Ok(Some(entry.clone())),
            Some(None) => Ok(None),
            None => utxo.get_utxo(op).map_err(ChainError::DbError),
        })
    }

    pub fn get_utxo(&self, outpoint: &OutPoint) -> Result<Option<UtxoEntry>, String> {
        self.utxo_store.get_utxo(outpoint)
    }

    pub fn get_block(&self, hash: &Hash256) -> Option<&Block> {
        self.blocks.peek(hash).map(|data| &data.block)
    }

    pub fn get_block_by_height(&self, height: u64) -> Option<&Block> {
        // Use storage for lookup, then check cache
        if let Ok(Some(block)) = self.block_store.get_block_by_height(height) {
            let hash = block.hash();
            return self.blocks.peek(&hash).map(|data| &data.block);
        }
        None
    }

    pub fn get_tip(&self) -> Result<Option<BlockData>, String> {
        match self.tip {
            Some(hash) => {
                let data = self.blocks.peek(&hash).cloned();
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
            if let Some(data) = self.blocks.peek(&current) {
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
        // Use canonical DB index directly — avoids walking the cache.
        self.block_store.get_hash_by_height(height).ok().flatten()
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
        self.blocks.peek(hash).map(|d| d.height)
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
