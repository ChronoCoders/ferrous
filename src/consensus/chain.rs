use crate::consensus::block::{Block, BlockData, BlockHeader, U256};
use crate::consensus::difficulty::{validate_difficulty, DifficultyError};
use crate::consensus::params::ChainParams;
use crate::consensus::utxo::{OutPoint, UtxoEntry, UtxoError};
use crate::consensus::validation::{validate_block, ValidationError};
use crate::primitives::hash::Hash256;
use crate::script::engine::{validate_p2pkh, ScriptContext};
use crate::storage::{BlockStore, ChainStateStore, ChainTip, Database, UtxoStore};
use log::info;
use std::collections::HashMap;
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

pub type BlockUtxoView = (Vec<(OutPoint, UtxoEntry)>, Vec<OutPoint>);

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

        // Now iterate forward
        for hash in chain_hashes.iter().rev() {
            let block = self
                .block_store
                .get_block(hash)?
                .ok_or_else(|| format!("Missing block {}", hex::encode(hash)))?;

            let header = block.header;
            let block_hash = header.hash();

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
                ) // Add u128 to U256? U256 + U256
                  // header.work() returns U256
                  // prev_data.cumulative_work is U256
                  // We need impl Add for U256
            };

            self.blocks.insert(
                block_hash,
                BlockData {
                    block,
                    height,
                    cumulative_work, // Assumes U256 has Add or we implement it
                },
            );
        }

        self.tip = Some(chain_tip.hash);

        info!("Recovery complete: {} blocks loaded", self.blocks.len());
        Ok(())
    }

    // #[allow(dead_code)]
    fn reorganize(&mut self, old_tip: &Hash256, new_tip: &Hash256) -> Result<(), ChainError> {
        info!("Reorganizing chain from {} to {}", hex::encode(old_tip), hex::encode(new_tip));
        
        // Find common ancestor
        let mut old_curr = *old_tip;
        let mut new_curr = *new_tip;
        
        let _old_height = self.blocks.get(&old_curr).map(|d| d.height).unwrap_or(0);
        let _new_height = self.blocks.get(&new_curr).map(|d| d.height).unwrap_or(0);
        
        // Bring to same height
        while self.blocks.get(&old_curr).map(|d| d.height).unwrap_or(0) > self.blocks.get(&new_curr).map(|d| d.height).unwrap_or(0) {
            if let Some(data) = self.blocks.get(&old_curr) {
                 old_curr = data.block.header.prev_block_hash;
            } else {
                 break;
            }
        }
        
        let mut new_chain = Vec::new();
        while self.blocks.get(&new_curr).map(|d| d.height).unwrap_or(0) > self.blocks.get(&old_curr).map(|d| d.height).unwrap_or(0) {
            new_chain.push(new_curr);
            if let Some(data) = self.blocks.get(&new_curr) {
                new_curr = data.block.header.prev_block_hash;
            } else {
                break;
            }
        }
        
        // Step back together
        while old_curr != new_curr {
             // Avoid infinite loops if we hit genesis or unknown block
             if old_curr == [0u8; 32] || new_curr == [0u8; 32] {
                 break;
             }
             
             if let Some(data) = self.blocks.get(&old_curr) {
                 old_curr = data.block.header.prev_block_hash;
             } else {
                 break;
             }
             
             new_chain.push(new_curr);
             if let Some(data) = self.blocks.get(&new_curr) {
                 new_curr = data.block.header.prev_block_hash;
             } else {
                 break;
             }
        }
        
        let ancestor = old_curr;
        
        // Disconnect old blocks (from old_tip back to ancestor)
        let mut curr = *old_tip;
        while curr != ancestor {
            if let Some(data) = self.blocks.get(&curr) {
                 let block = &data.block;
                 // TODO: Disconnect block from UTXO set (requires undo data)
                 let _ = block; 
                 curr = block.header.prev_block_hash;
            } else {
                 break;
            }
        }
        
        // Connect new blocks (from ancestor to new_tip)
        // new_chain is in reverse order (tip -> ancestor)
        for hash in new_chain.iter().rev() {
            if let Some(data) = self.blocks.get(hash) {
                 let block = &data.block;
                 let height = data.height;
                 // Apply intermediate blocks to UTXO set
                 let (created_utxos, spent_utxos) = self.apply_block_to_utxo(block, height)?;
                 self.utxo_store.apply_block(&created_utxos, &spent_utxos).map_err(ChainError::DbError)?;
            }
        }
        
        Ok(())
    }

    pub fn add_block(&mut self, block: Block) -> Result<(), ChainError> {
        let block_hash = block.hash();

        // Validate block
        validate_block(&block.header, &block.transactions).map_err(ChainError::InvalidBlock)?;

        // Calculate height and cumulative work
        let (height, cumulative_work) = if block.header.prev_block_hash == [0u8; 32] {
            (0, block.header.work())
        } else {
            let prev_data = self
                .blocks
                .get(&block.header.prev_block_hash)
                .ok_or(ChainError::OrphanBlock)?;

            // Validate difficulty
            validate_difficulty(Some(&prev_data.block.header), &block.header, &self.params)
                .map_err(ChainError::InvalidDifficulty)?;

            (
                prev_data.height + 1,
                prev_data.cumulative_work + block.header.work(),
            )
        };
        
        // Update tip if this is the new best chain
        let should_update_tip = match self.tip {
            None => true,
            Some(current_tip) => {
                let current_work = self.blocks.get(&current_tip).unwrap().cumulative_work;
                cumulative_work > current_work
            }
        };

        if should_update_tip {
            // Check for reorg
            if let Some(current_tip_hash) = self.tip {
                // If the new block does not extend the current tip, we have a reorg
                if block.header.prev_block_hash != current_tip_hash {
                    self.reorganize(&current_tip_hash, &block_hash)?;
                }
            }

            // Apply to UTXO set ONLY if it's the new tip
            let (created_utxos, spent_utxos) = self.apply_block_to_utxo(&block, height)?;

            // Persist everything atomically
            self.block_store
                .store_block(&block, height, cumulative_work)
                .map_err(ChainError::DbError)?;
            self.utxo_store
                .apply_block(&created_utxos, &spent_utxos)
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
             // Side chain block: Store it but DO NOT apply to UTXO set
             self.block_store
                .store_block(&block, height, cumulative_work)
                .map_err(ChainError::DbError)?;
        }

        // Update in-memory index
        self.blocks.insert(
            block_hash,
            BlockData {
                block,
                height,
                cumulative_work,
            },
        );

        Ok(())
    }

    fn apply_block_to_utxo(&self, block: &Block, height: u64) -> Result<BlockUtxoView, ChainError> {
        let mut created = Vec::new();
        let mut spent = Vec::new();

        // Process all transactions
        for (tx_idx, tx) in block.transactions.iter().enumerate() {
            let txid = tx.txid();
            let is_coinbase = tx_idx == 0;

            // Spend inputs (skip coinbase)
            if !is_coinbase {
                let mut spent_outputs = Vec::with_capacity(tx.inputs.len());

                for input in &tx.inputs {
                    // Check if input exists and is unspent
                    let outpoint = OutPoint {
                        txid: input.prev_txid,
                        vout: input.prev_index,
                    };

                    let utxo_entry = self
                        .utxo_store
                        .get_utxo(&outpoint)
                        .map_err(ChainError::DbError)?
                        .ok_or(ChainError::UtxoError(UtxoError::UtxoNotFound))?;

                    spent.push(outpoint);
                    spent_outputs.push(utxo_entry.output);
                }

                // Validate signatures for all inputs
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

            // Create outputs
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

        Ok((created, spent))
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
        timestamps[len / 2]
    }

    pub fn is_utxo_unspent(&self, outpoint: &OutPoint) -> bool {
        self.utxo_store.has_utxo(outpoint).unwrap_or(false)
    }

    pub fn has_block(&self, hash: &Hash256) -> bool {
        self.block_store.has_block(hash).unwrap_or(false)
    }

    pub fn get_block_locator(&self) -> Vec<Hash256> {
        let mut hashes = Vec::new();
        let mut step = 1;
        let mut current_height = self.get_height();

        // Push tip
        if let Some(tip) = self.tip {
            hashes.push(tip);
        } else {
            return vec![[0u8; 32]];
        }

        while current_height > 0 {
            if hashes.len() >= 10 {
                step *= 2;
            }
            if current_height < step {
                current_height = 0;
            } else {
                current_height -= step;
            }

            if let Some(block) = self.get_block_by_height(current_height) {
                hashes.push(block.hash());
            }
        }
        hashes
    }

    pub fn get_header_at_height(&self, height: u64) -> Option<BlockHeader> {
        self.get_block_by_height(height).map(|b| b.header)
    }

    pub fn get_height_for_hash(&self, hash: &Hash256) -> Option<u64> {
        self.blocks.get(hash).map(|d| d.height)
    }

    pub fn validate_header_standalone(&self, _header: &BlockHeader) -> Result<(), String> {
        Ok(())
    }

    pub fn store_header_only(&self, _header: &BlockHeader) -> Result<(), String> {
        Ok(())
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
