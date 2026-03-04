use crate::consensus::block::{BlockData, BlockHeader};
use crate::consensus::difficulty::{validate_difficulty, DifficultyError};
use crate::consensus::merkle::compute_merkle_root;
use crate::consensus::params::ChainParams;
use crate::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
use crate::consensus::utxo::{OutPoint, UtxoEntry, UtxoError, UtxoSet};
use crate::consensus::validation::{validate_block, ValidationError};
use crate::primitives::hash::Hash256;
use crate::storage::BlockchainDB;
use num_bigint::BigUint;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainError {
    BlockNotFound,
    InvalidBlock(ValidationError),
    InvalidDifficulty(DifficultyError),
    UtxoError(UtxoError),
    OrphanBlock,
    DbError(String),
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
    db: Arc<BlockchainDB>,
    params: ChainParams,
    tip_hash: Hash256,
    tip_height: u32,
    header_tip_hash: Hash256,
    header_tip_height: u32,
    utxo_set: UtxoSet,
}

impl ChainState {
    pub fn new(
        params: ChainParams,
        db_path: &str,
        genesis_override: Option<(BlockHeader, Transaction)>,
    ) -> Result<Self, String> {
        let db = Arc::new(BlockchainDB::open(db_path)?);

        // Check if we have existing data
        if let Some((tip_hash, tip_height)) = db.get_tip()? {
            // Load from existing database
            println!("Loading existing chain from height {}", tip_height);
            // TODO: Load header tip from DB if persisted, otherwise assume same as block tip
            // For now assuming same as block tip, which means we might redownload headers if we were ahead
            let header_tip_hash = tip_hash;
            let header_tip_height = tip_height;

            let utxo_set = db.load_utxo_set()?;

            Ok(Self {
                db,
                params,
                tip_hash,
                tip_height,
                header_tip_hash,
                header_tip_height,
                utxo_set,
            })
        } else {
            // Initialize with genesis
            println!("Initializing new chain with genesis block");
            let (genesis, genesis_tx) = genesis_override.unwrap_or_else(create_genesis_block);
            let genesis_hash = genesis.hash();
            let genesis_work = calculate_work(&genesis);

            // Create BlockData
            let block_data = BlockData {
                header: genesis,
                transactions: vec![genesis_tx.clone()],
                height: 0,
                cumulative_work: genesis_work,
            };

            // Store genesis
            db.put_header(&genesis_hash, &genesis)?;
            db.put_block(&genesis_hash, &block_data)?;
            db.put_height_index(0, &genesis_hash)?;
            db.set_tip(&genesis_hash, 0)?;

            // Create genesis UTXO
            let mut utxo_set = UtxoSet::new();
            utxo_set
                .add_transaction(&genesis_tx, 0, true)
                .expect("genesis UTXO add must succeed");
            // Persist genesis UTXO
            for (idx, output) in genesis_tx.outputs.iter().enumerate() {
                let outpoint = OutPoint {
                    txid: genesis_tx.txid(),
                    index: idx as u32,
                };
                let entry = UtxoEntry {
                    output: output.clone(),
                    height: 0,
                    is_coinbase: true,
                };
                db.put_utxo(&outpoint, &entry)?;
            }

            Ok(Self {
                db,
                params,
                tip_hash: genesis_hash,
                tip_height: 0,
                header_tip_hash: genesis_hash,
                header_tip_height: 0,
                utxo_set,
            })
        }
    }

    pub fn add_block(
        &mut self,
        header: BlockHeader,
        transactions: Vec<Transaction>,
    ) -> Result<bool, ChainError> {
        let block_hash = header.hash();

        // Get parent
        let parent_header = self
            .db
            .get_header(&header.prev_block_hash)?
            .ok_or(ChainError::OrphanBlock)?;
        let parent_block = self
            .db
            .get_block(&header.prev_block_hash)?
            .ok_or(ChainError::OrphanBlock)?;

        let height = parent_block.height + 1;

        // Validate difficulty
        validate_difficulty(Some(&parent_header), &header, &self.params)
            .map_err(ChainError::InvalidDifficulty)?;

        // Validate block structure
        validate_block(&header, &transactions).map_err(ChainError::InvalidBlock)?;

        // Calculate cumulative work
        let work = calculate_work(&header);
        let cumulative_work = parent_block.cumulative_work + work;

        // Create BlockData
        let block_data = BlockData {
            header,
            transactions: transactions.clone(),
            height,
            cumulative_work,
        };

        // Store block
        self.db.put_header(&block_hash, &header)?;
        self.db.put_block(&block_hash, &block_data)?;
        // Note: We don't update height index or tip yet, unless it's the new tip.
        // But if we don't store it, we can't find it later if it becomes tip?
        // We stored it in BLOCKS and HEADERS. HEIGHT_INDEX is for the main chain.

        // Check if new tip
        if cumulative_work > self.get_tip_work()? {
            self.reorganize(block_hash)?;
            Ok(true) // New tip
        } else {
            Ok(false) // Valid but not tip
        }
    }

    fn reorganize(&mut self, new_tip: Hash256) -> Result<(), ChainError> {
        // Get new chain path to genesis
        let new_chain = self.get_chain_to_genesis(new_tip)?;

        // Rebuild UTXO from scratch (simple implementation)
        self.utxo_set = UtxoSet::new();
        self.db.clear_utxos()?;

        // Apply new chain from genesis
        // new_chain is [tip, ..., genesis]
        // new_chain.iter().rev() is [genesis, ..., tip]
        for &hash in new_chain.iter().rev() {
            let block = self.db.get_block(&hash)?.ok_or(ChainError::BlockNotFound)?;
            // Apply transactions to memory UTXO set
            for (i, tx) in block.transactions.iter().enumerate() {
                let is_coinbase = i == 0;
                self.utxo_set
                    .apply_transaction(tx, block.height, is_coinbase)
                    .map_err(ChainError::UtxoError)?;
                // Update DB UTXO set
                // 1. Add new outputs
                for (idx, output) in tx.outputs.iter().enumerate() {
                    let outpoint = OutPoint {
                        txid: tx.txid(),
                        index: idx as u32,
                    };
                    let entry = UtxoEntry {
                        output: output.clone(),
                        height: block.height,
                        is_coinbase,
                    };
                    self.db.put_utxo(&outpoint, &entry)?;
                }

                // 2. Remove spent inputs
                if !is_coinbase {
                    for input in &tx.inputs {
                        let outpoint = OutPoint {
                            txid: input.prev_txid,
                            index: input.prev_index,
                        };
                        self.db.delete_utxo(&outpoint)?;
                    }
                }
            }
        }

        // Update tip
        let new_block = self
            .db
            .get_block(&new_tip)?
            .ok_or(ChainError::BlockNotFound)?;
        self.tip_hash = new_tip;
        self.tip_height = new_block.height;
        self.db.set_tip(&new_tip, self.tip_height)?;

        // Rebuild height index
        // Ideally we should only update changed heights, but full rebuild is safe
        // Actually, we only need to update the main chain.
        // new_chain contains the main chain.
        for &hash in new_chain.iter().rev() {
            let block = self.db.get_block(&hash)?.ok_or(ChainError::BlockNotFound)?;
            self.db.put_height_index(block.height, &hash)?;
        }

        Ok(())
    }

    fn get_chain_to_genesis(&self, start: Hash256) -> Result<Vec<Hash256>, ChainError> {
        let mut chain = vec![start];
        let mut current = start;

        loop {
            let block = self
                .db
                .get_header(&current)?
                .ok_or(ChainError::BlockNotFound)?;
            if block.prev_block_hash == [0u8; 32] {
                // Genesis reached
                break;
            }
            // Check if we are at height 0 (genesis) by checking if we have a block data
            // But checking prev_block_hash is simpler for genesis detection.
            current = block.prev_block_hash;
            chain.push(current);
        }

        Ok(chain)
    }

    fn get_tip_work(&self) -> Result<u128, ChainError> {
        let block = self
            .db
            .get_block(&self.tip_hash)?
            .ok_or(ChainError::BlockNotFound)?;
        Ok(block.cumulative_work)
    }

    pub fn get_tip(&self) -> Result<BlockData, ChainError> {
        self.db
            .get_block(&self.tip_hash)?
            .ok_or(ChainError::BlockNotFound)
    }

    pub fn has_block(&self, hash: &Hash256) -> bool {
        self.db.get_header(hash).unwrap_or(None).is_some()
    }

    pub fn get_block_by_hash(&self, hash: &Hash256) -> Option<(BlockHeader, Vec<Transaction>)> {
        match self.db.get_block(hash) {
            Ok(Some(block_data)) => Some((block_data.header, block_data.transactions)),
            _ => None,
        }
    }

    pub fn get_block(&self, hash: &Hash256) -> Result<Option<BlockData>, ChainError> {
        self.db.get_block(hash).map_err(ChainError::DbError)
    }

    pub fn get_block_at_height(&self, height: u32) -> Result<Option<BlockData>, ChainError> {
        if let Some(hash) = self.db.get_hash_at_height(height)? {
            self.db.get_block(&hash).map_err(ChainError::DbError)
        } else {
            Ok(None)
        }
    }

    pub fn get_header_at_height(&self, height: u32) -> Option<BlockHeader> {
        match self.db.get_hash_at_height(height) {
            Ok(Some(hash)) => self.db.get_header(&hash).unwrap_or(None),
            _ => None,
        }
    }

    pub fn get_height_for_hash(&self, hash: &Hash256) -> Option<u32> {
        self.db.get_block(hash).ok().flatten().map(|b| b.height)
    }

    pub fn is_utxo_unspent(&self, outpoint: &OutPoint) -> bool {
        self.utxo_set.utxos.contains_key(outpoint)
    }

    pub fn get_block_locator(&self) -> Vec<Hash256> {
        let mut locator = Vec::new();
        // Use header tip to include potentially downloaded but unverified (block-wise) headers
        let tip_height = self.header_tip_height;

        let mut step = 1;
        let mut height = tip_height;

        loop {
            if let Some(header) = self.get_header_at_height(height) {
                locator.push(header.hash());
            }

            if height == 0 {
                break;
            }

            if locator.len() >= 10 {
                step *= 2;
            }

            height = height.saturating_sub(step);
        }
        locator
    }

    pub fn validate_header_standalone(&self, header: &BlockHeader) -> Result<bool, ChainError> {
        // 1. Check PoW
        match header.check_proof_of_work() {
            Ok(true) => Ok(true),
            Ok(false) => Ok(false),
            Err(e) => Err(ChainError::InvalidDifficulty(
                crate::consensus::difficulty::DifficultyError::TargetError(e),
            )),
        }
    }

    pub fn store_header_only(&mut self, header: &BlockHeader) -> Result<(), ChainError> {
        let hash = header.hash();

        // Store header in DB
        self.db.put_header(&hash, header)?;

        // Check if this header extends our header chain
        if header.prev_block_hash == self.header_tip_hash {
            // It's the next header
            self.header_tip_height += 1;
            self.header_tip_hash = hash;

            // Update height index so get_header_at_height works
            // Note: This might make get_block_at_height return None for this height, which is correct
            self.db.put_height_index(self.header_tip_height, &hash)?;
        } else {
            // It might be a fork or out of order.
            // For simple headers-first sync, we assume we receive them in order.
            // If we receive a header that doesn't link to tip, we might have a gap or fork.
            // Dealing with forks in headers-only mode requires more complex logic (HeaderChain struct).
            // For this task, we'll assume linear sync or just store it without updating tip if it doesn't link.
            // But if it doesn't link, get_block_locator won't see it next time.

            // If it links to something else, maybe we should check if it has more work?
            // For now, let's keep it simple: only update tip if it extends current tip.
        }

        Ok(())
    }

    pub fn export_utxos(&self) -> Result<Vec<(OutPoint, UtxoEntry)>, ChainError> {
        self.db.load_utxo_set_raw().map_err(ChainError::DbError)
    }

    pub fn median_time_past(&self) -> u64 {
        // This function is often called in tight loops or checks, so we panic on DB error for simplicity
        // in this example, or return 0.
        // Ideally should return Result. But changing signature affects many callers.
        // Let's try to return Result or unwrap.
        // The original signature returned u64.
        let mut timestamps = Vec::new();
        let mut current = self.tip_hash;

        for _ in 0..11 {
            if let Ok(Some(header)) = self.db.get_header(&current) {
                timestamps.push(header.timestamp);
                if header.prev_block_hash == [0u8; 32] {
                    break;
                }
                current = header.prev_block_hash;
            } else {
                break;
            }
        }

        if timestamps.is_empty() {
            return 0;
        }

        timestamps.sort_unstable();

        let len = timestamps.len();
        if len & 1 == 0 {
            timestamps[len / 2 - 1]
        } else {
            timestamps[len / 2]
        }
    }
}

fn calculate_work(header: &BlockHeader) -> u128 {
    let target = header.target().unwrap();

    // Convert target from little-endian bytes to BigUint
    let target_big = BigUint::from_bytes_le(&target.0);

    // Work = 2^256 / (target + 1)
    let numerator = BigUint::from(1u8) << 256;
    let denominator = target_big + 1u8;

    let work_big: BigUint = numerator / denominator;

    // Convert result to u128 (use low 128 bits)
    let bytes = work_big.to_bytes_le();
    let mut result = 0u128;

    for (i, &byte) in bytes.iter().enumerate().take(16) {
        result |= (byte as u128) << (8 * i);
    }

    result
}

fn create_genesis_block() -> (BlockHeader, Transaction) {
    let coinbase = Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig: vec![0x04, 0x00, 0x00, 0x00, 0x00], // height 0
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value: 50 * 100_000_000,
            script_pubkey: vec![0x51], // OP_1
        }],
        witnesses: vec![Witness {
            stack_items: Vec::new(),
        }],
        locktime: 0,
    };

    let txids = vec![coinbase.txid()];
    let merkle_root = compute_merkle_root(&txids);

    let mut header = BlockHeader {
        version: 1,
        prev_block_hash: [0u8; 32],
        merkle_root,
        timestamp: 1_700_000_000,
        n_bits: 0x207f_ffff, // Easy testnet difficulty
        nonce: 0,
    };

    while !header.check_proof_of_work().unwrap_or(false) {
        header.nonce = header.nonce.wrapping_add(1);
    }

    (header, coinbase)
}
