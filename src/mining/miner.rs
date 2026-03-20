use crate::consensus::block::BlockHeader;
use crate::consensus::chain::ChainState;
use crate::consensus::difficulty::{calculate_next_target, u256_to_compact};
use crate::consensus::merkle::compute_merkle_root;
use crate::consensus::params::ChainParams;
use crate::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

const MAX_FUTURE_OFFSET: u64 = 7200;

/// Pre-solved block template — everything needed to run PoW, with no chain
/// reference held. Build with `Miner::build_template`, solve with
/// `Miner::solve_template`.
#[derive(Clone)]
pub struct BlockTemplate {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct Miner {
    pub params: ChainParams,
    pub mining_address: Vec<u8>,
    pub event_sender: Option<Sender<MiningEvent>>,
    stats: Arc<MinerStats>,
}

#[derive(Debug)]
struct MinerStats {
    total_hashes: AtomicU64,
    total_micros: AtomicU64,
    active: AtomicBool,
    active_hashes: AtomicU64,
    active_start_micros: AtomicU64,
}

fn now_micros() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    now.min(u64::MAX as u128) as u64
}

#[derive(Debug, Clone)]
pub struct MiningEvent {
    pub height: u32,
    pub nonce: u64,
    pub worker_id: u64,
    pub hash: String,
    pub hashes_tried: u64,
    pub elapsed_secs: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MiningError {
    ChainError(String),
    InvalidCoinbase,
    InvalidTimestamp,
}

#[cfg(not(test))]
fn network_adjusted_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[cfg(test)]
mod time {
    use std::cell::Cell;

    thread_local! {
        pub static MOCK_TIME: Cell<Option<u64>> = const { Cell::new(None) };
    }

    pub fn set_mock_time(t: Option<u64>) {
        MOCK_TIME.with(|cell| cell.set(t));
    }

    pub fn now() -> u64 {
        MOCK_TIME.with(|cell| cell.get()).unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        })
    }
}

#[cfg(test)]
fn network_adjusted_time() -> u64 {
    time::now()
}

pub fn next_block_timestamp(chain: &ChainState) -> Result<u64, MiningError> {
    let mtp = chain.median_time_past();
    let min_time = mtp.checked_add(1).ok_or(MiningError::InvalidTimestamp)?;

    let nat = network_adjusted_time();

    let candidate = if nat > min_time { nat } else { min_time };

    let max_allowed = nat
        .checked_add(MAX_FUTURE_OFFSET)
        .ok_or(MiningError::InvalidTimestamp)?;

    if candidate > max_allowed {
        return Err(MiningError::InvalidTimestamp);
    }

    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
    use crate::primitives::hash::Hash256;
    use tempfile::TempDir;

    fn make_genesis(timestamp: u64, n_bits: u32) -> (BlockHeader, Transaction) {
        let coinbase = Transaction {
            version: 1,
            inputs: vec![TxInput {
                prev_txid: [0u8; 32],
                prev_index: 0xFFFF_FFFF,
                script_sig: vec![0x04, 0x00, 0x00, 0x00, 0x00],
                sequence: 0xFFFF_FFFF,
            }],
            outputs: vec![TxOutput {
                value: 50 * 100_000_000,
                script_pubkey: vec![0x51],
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
            timestamp,
            n_bits,
            nonce: 0,
        };

        while !header.check_proof_of_work().unwrap() {
            header.nonce += 1;
        }

        (header, coinbase)
    }

    fn single_block_chain(genesis_time: u64) -> (ChainState, TempDir) {
        let (genesis, tx) = make_genesis(genesis_time, 0x207f_ffff);
        let temp_dir = TempDir::new().unwrap();
        let mut chain = ChainState::new(
            crate::consensus::params::Network::Regtest.params(),
            temp_dir.path(),
        )
        .unwrap();

        use crate::consensus::block::Block;
        let block = Block {
            header: genesis,
            transactions: vec![tx],
        };
        chain.add_block(block).unwrap();

        (chain, temp_dir)
    }

    #[test]
    fn next_timestamp_nat_less_than_mtp() {
        let nat = 1_000_000_000u64;
        let mtp = nat + 10;

        time::set_mock_time(Some(nat));

        let (chain, _tmp) = single_block_chain(mtp);

        let ts = next_block_timestamp(&chain).expect("timestamp ok");

        assert_eq!(ts, mtp + 1);
    }

    #[test]
    fn next_timestamp_nat_equal_mtp() {
        let nat = 1_000_000_000u64;
        let mtp = nat;

        time::set_mock_time(Some(nat));

        let (chain, _tmp) = single_block_chain(mtp);

        let ts = next_block_timestamp(&chain).expect("timestamp ok");

        assert_eq!(ts, mtp + 1);
    }

    #[test]
    fn next_timestamp_nat_greater_than_mtp() {
        let nat = 1_000_000_000u64;
        let mtp = nat - 10;

        time::set_mock_time(Some(nat));

        let (chain, _tmp) = single_block_chain(mtp);

        let ts = next_block_timestamp(&chain).expect("timestamp ok");

        assert_eq!(ts, nat);
    }

    #[test]
    fn next_timestamp_too_far_in_future() {
        let nat = 1_000_000_000u64;
        let mtp = nat + MAX_FUTURE_OFFSET + 100;

        time::set_mock_time(Some(nat));

        let (chain, _tmp) = single_block_chain(mtp);

        let err = next_block_timestamp(&chain).unwrap_err();

        assert_eq!(err, MiningError::InvalidTimestamp);
    }

    #[test]
    fn mined_blocks_respect_timestamp_rules() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        time::set_mock_time(Some(now));

        let (genesis, tx) = make_genesis(now.saturating_sub(10), 0x207f_ffff);
        let temp_dir = TempDir::new().unwrap();
        let mut chain = ChainState::new(
            crate::consensus::params::Network::Regtest.params(),
            temp_dir.path(),
        )
        .unwrap();

        use crate::consensus::block::Block;
        let block = Block {
            header: genesis,
            transactions: vec![tx],
        };
        chain.add_block(block).unwrap();

        let miner = Miner::new(
            crate::consensus::params::Network::Regtest.params(),
            vec![0x51],
        );

        for i in 0..10 {
            time::set_mock_time(Some(now + i as u64));
            miner
                .mine_and_attach(&mut chain, Vec::new())
                .expect("mine ok");
        }

        let tip = chain.get_tip().unwrap().unwrap();
        let mut prev_headers = Vec::new();
        let mut current_hash: Hash256 = tip.block.header.prev_block_hash;

        while let Some(block) = chain.get_block(&current_hash) {
            prev_headers.push(block.header);
            if prev_headers.len() == 11 || block.header.prev_block_hash == [0u8; 32] {
                // block.height not available in Block
                break;
            }
            current_hash = block.header.prev_block_hash;
        }

        let res =
            crate::consensus::validation::validate_timestamp(&tip.block.header, &prev_headers);
        match res {
            Ok(()) => {}
            Err(e) => panic!("timestamp validation failed: {:?}", e),
        }
    }
}

impl Miner {
    pub fn new(params: ChainParams, mining_address: Vec<u8>) -> Self {
        Self {
            params,
            mining_address,
            event_sender: None,
            stats: Arc::new(MinerStats {
                total_hashes: AtomicU64::new(0),
                total_micros: AtomicU64::new(0),
                active: AtomicBool::new(false),
                active_hashes: AtomicU64::new(0),
                active_start_micros: AtomicU64::new(0),
            }),
        }
    }

    pub fn with_event_sender(mut self, sender: Sender<MiningEvent>) -> Self {
        self.event_sender = Some(sender);
        self
    }

    pub fn hashrate_hps(&self) -> f64 {
        if self.stats.active.load(Ordering::Relaxed) {
            let start = self.stats.active_start_micros.load(Ordering::Relaxed);
            let elapsed_micros = now_micros().saturating_sub(start) as f64;
            if elapsed_micros <= 0.0 {
                return 0.0;
            }
            let hashes = self.stats.active_hashes.load(Ordering::Relaxed) as f64;
            return hashes / (elapsed_micros / 1_000_000.0);
        }

        let hashes = self.stats.total_hashes.load(Ordering::Relaxed) as f64;
        let micros = self.stats.total_micros.load(Ordering::Relaxed) as f64;
        if micros <= 0.0 {
            return 0.0;
        }
        hashes / (micros / 1_000_000.0)
    }

    pub fn create_coinbase_internal(
        &self,
        height: u32,
        subsidy: u64,
        fees: u64,
        script_pubkey: Vec<u8>,
    ) -> Transaction {
        // Encode height in script_sig (BIP34 style: length + LE bytes)
        let mut script_sig = Vec::new();
        let height_bytes = height.to_le_bytes();
        script_sig.push(4);
        script_sig.extend_from_slice(&height_bytes);

        Transaction {
            version: 1,
            inputs: vec![TxInput {
                prev_txid: [0u8; 32],
                prev_index: 0xFFFF_FFFF,
                script_sig,
                sequence: 0xFFFF_FFFF,
            }],
            outputs: vec![TxOutput {
                value: subsidy + fees,
                script_pubkey,
            }],
            witnesses: vec![Witness {
                stack_items: Vec::new(),
            }],
            locktime: 0,
        }
    }

    fn mine_block_with_script(
        &self,
        chain: &ChainState,
        transactions: Vec<Transaction>,
        script_pubkey: Vec<u8>,
    ) -> Result<(BlockHeader, Vec<Transaction>), MiningError> {
        let tip = chain
            .get_tip()
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?
            .ok_or(MiningError::ChainError("Chain tip not found".to_string()))?;
        let height = (tip.height + 1) as u32;

        let subsidy = crate::consensus::validation::calculate_subsidy(height);
        let total_fees = 0u64;

        let coinbase = self.create_coinbase_internal(height, subsidy, total_fees, script_pubkey);

        let mut all_txs = vec![coinbase];
        all_txs.extend(transactions);

        let txids: Vec<_> = all_txs.iter().map(|tx| tx.txid()).collect();
        let merkle_root = compute_merkle_root(&txids);

        let timestamp = next_block_timestamp(chain)?;

        let target = calculate_next_target(&tip.block.header, timestamp, &self.params)
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?;

        let n_bits = u256_to_compact(&target);

        let prev_block_hash = tip.block.header.hash();

        let header = BlockHeader {
            version: 1,
            prev_block_hash,
            merkle_root,
            timestamp,
            n_bits,
            nonce: 0,
        };

        self.stats.active.store(true, Ordering::Relaxed);
        self.stats.active_hashes.store(0, Ordering::Relaxed);
        self.stats
            .active_start_micros
            .store(now_micros(), Ordering::Relaxed);

        let num_workers = std::env::var("FERROUS_MINER_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or_else(num_cpus::get);
        let found = Arc::new(AtomicBool::new(false));
        let solution_nonce = Arc::new(AtomicU64::new(0));
        let solution_timestamp = Arc::new(AtomicU64::new(timestamp));
        let solution_worker = Arc::new(AtomicU64::new(0));

        let nonce_range = u64::MAX / num_workers as u64;
        let mining_start = std::time::Instant::now();

        (0..num_workers).into_par_iter().for_each(|worker_id| {
            let mut local_header = header;
            let start_nonce = worker_id as u64 * nonce_range;
            let mut current_nonce = start_nonce;
            let mut iterations = 0u64;

            loop {
                if found.load(Ordering::Relaxed) {
                    break;
                }

                local_header.nonce = current_nonce;

                if local_header.check_proof_of_work().unwrap_or(false) {
                    found.store(true, Ordering::Relaxed);
                    solution_nonce.store(current_nonce, Ordering::Relaxed);
                    solution_timestamp.store(local_header.timestamp, Ordering::Relaxed);
                    solution_worker.store(worker_id as u64, Ordering::Relaxed);
                    break;
                }

                current_nonce = current_nonce.wrapping_add(1);
                iterations += 1;
                self.stats.active_hashes.fetch_add(1, Ordering::Relaxed);

                if iterations.is_multiple_of(50_000) {
                    std::thread::yield_now();
                }
                if iterations.is_multiple_of(200_000) {
                    std::thread::sleep(Duration::from_millis(1));
                }

                if current_nonce == start_nonce.wrapping_add(nonce_range) {
                    current_nonce = start_nonce;
                }
            }
        });

        let elapsed_secs = mining_start.elapsed().as_secs_f64();
        let hashes_tried = self.stats.active_hashes.load(Ordering::Relaxed);
        let elapsed_micros = (elapsed_secs * 1_000_000.0).round();
        if elapsed_micros.is_finite() && elapsed_micros > 0.0 {
            self.stats
                .total_hashes
                .fetch_add(hashes_tried, Ordering::Relaxed);
            self.stats.total_micros.fetch_add(
                elapsed_micros.min(u64::MAX as f64) as u64,
                Ordering::Relaxed,
            );
        }
        self.stats.active.store(false, Ordering::Relaxed);

        let final_header = BlockHeader {
            version: header.version,
            prev_block_hash: header.prev_block_hash,
            merkle_root: header.merkle_root,
            timestamp: solution_timestamp.load(Ordering::Relaxed),
            n_bits: header.n_bits,
            nonce: solution_nonce.load(Ordering::Relaxed),
        };

        if let Some(sender) = &self.event_sender {
            let event = MiningEvent {
                height,
                nonce: solution_nonce.load(Ordering::Relaxed),
                worker_id: solution_worker.load(Ordering::Relaxed),
                hash: hex::encode(final_header.hash()),
                hashes_tried,
                elapsed_secs,
            };
            let _ = sender.send(event);
        }

        if !final_header
            .check_proof_of_work()
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?
        {
            return Err(MiningError::ChainError(
                "PoW verification failed".to_string(),
            ));
        }

        Ok((final_header, all_txs))
    }

    /// Build a block template from the current chain tip. Reads chain state and
    /// returns immediately — does NOT run PoW. Callers should release any chain
    /// lock before calling `solve_template`.
    pub fn build_template(
        &self,
        chain: &ChainState,
        transactions: Vec<Transaction>,
    ) -> Result<BlockTemplate, MiningError> {
        self.build_template_with_script(chain, transactions, self.mining_address.clone())
    }

    fn build_template_with_script(
        &self,
        chain: &ChainState,
        transactions: Vec<Transaction>,
        script_pubkey: Vec<u8>,
    ) -> Result<BlockTemplate, MiningError> {
        let tip = chain
            .get_tip()
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?
            .ok_or(MiningError::ChainError("Chain tip not found".to_string()))?;
        let height = (tip.height + 1) as u32;

        let subsidy = crate::consensus::validation::calculate_subsidy(height);
        let total_fees = 0u64;

        let coinbase =
            self.create_coinbase_internal(height, subsidy, total_fees, script_pubkey);

        let mut all_txs = vec![coinbase];
        all_txs.extend(transactions);

        let txids: Vec<_> = all_txs.iter().map(|tx| tx.txid()).collect();
        let merkle_root = compute_merkle_root(&txids);

        let timestamp = next_block_timestamp(chain)?;

        let target = calculate_next_target(&tip.block.header, timestamp, &self.params)
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?;

        let n_bits = u256_to_compact(&target);
        let prev_block_hash = tip.block.header.hash();

        let header = BlockHeader {
            version: 1,
            prev_block_hash,
            merkle_root,
            timestamp,
            n_bits,
            nonce: 0,
        };

        Ok(BlockTemplate {
            header,
            transactions: all_txs,
            height,
        })
    }

    /// Run PoW on a pre-built template. Does not access chain state — safe to
    /// call without holding any chain lock.
    pub fn solve_template(
        &self,
        template: BlockTemplate,
    ) -> Result<(BlockHeader, Vec<Transaction>), MiningError> {
        let header = template.header;
        let height = template.height;
        let all_txs = template.transactions;

        self.stats.active.store(true, Ordering::Relaxed);
        self.stats.active_hashes.store(0, Ordering::Relaxed);
        self.stats
            .active_start_micros
            .store(now_micros(), Ordering::Relaxed);

        let num_workers = std::env::var("FERROUS_MINER_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or_else(num_cpus::get);
        let found = Arc::new(AtomicBool::new(false));
        let solution_nonce = Arc::new(AtomicU64::new(0));
        let solution_timestamp = Arc::new(AtomicU64::new(header.timestamp));
        let solution_worker = Arc::new(AtomicU64::new(0));

        let nonce_range = u64::MAX / num_workers as u64;
        let mining_start = std::time::Instant::now();

        (0..num_workers).into_par_iter().for_each(|worker_id| {
            let mut local_header = header;
            let start_nonce = worker_id as u64 * nonce_range;
            let mut current_nonce = start_nonce;
            let mut iterations = 0u64;

            loop {
                if found.load(Ordering::Relaxed) {
                    break;
                }

                local_header.nonce = current_nonce;

                if local_header.check_proof_of_work().unwrap_or(false) {
                    found.store(true, Ordering::Relaxed);
                    solution_nonce.store(current_nonce, Ordering::Relaxed);
                    solution_timestamp.store(local_header.timestamp, Ordering::Relaxed);
                    solution_worker.store(worker_id as u64, Ordering::Relaxed);
                    break;
                }

                current_nonce = current_nonce.wrapping_add(1);
                iterations += 1;
                self.stats.active_hashes.fetch_add(1, Ordering::Relaxed);

                if iterations.is_multiple_of(50_000) {
                    std::thread::yield_now();
                }
                if iterations.is_multiple_of(200_000) {
                    std::thread::sleep(Duration::from_millis(1));
                }

                if current_nonce == start_nonce.wrapping_add(nonce_range) {
                    current_nonce = start_nonce;
                }
            }
        });

        let elapsed_secs = mining_start.elapsed().as_secs_f64();
        let hashes_tried = self.stats.active_hashes.load(Ordering::Relaxed);
        let elapsed_micros = (elapsed_secs * 1_000_000.0).round();
        if elapsed_micros.is_finite() && elapsed_micros > 0.0 {
            self.stats
                .total_hashes
                .fetch_add(hashes_tried, Ordering::Relaxed);
            self.stats.total_micros.fetch_add(
                elapsed_micros.min(u64::MAX as f64) as u64,
                Ordering::Relaxed,
            );
        }
        self.stats.active.store(false, Ordering::Relaxed);

        let final_header = BlockHeader {
            version: header.version,
            prev_block_hash: header.prev_block_hash,
            merkle_root: header.merkle_root,
            timestamp: solution_timestamp.load(Ordering::Relaxed),
            n_bits: header.n_bits,
            nonce: solution_nonce.load(Ordering::Relaxed),
        };

        if let Some(sender) = &self.event_sender {
            let event = MiningEvent {
                height,
                nonce: solution_nonce.load(Ordering::Relaxed),
                worker_id: solution_worker.load(Ordering::Relaxed),
                hash: hex::encode(final_header.hash()),
                hashes_tried,
                elapsed_secs,
            };
            let _ = sender.send(event);
        }

        if !final_header
            .check_proof_of_work()
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?
        {
            return Err(MiningError::ChainError(
                "PoW verification failed".to_string(),
            ));
        }

        Ok((final_header, all_txs))
    }

    pub fn mine_block(
        &self,
        chain: &ChainState,
        transactions: Vec<Transaction>,
    ) -> Result<(BlockHeader, Vec<Transaction>), MiningError> {
        self.mine_block_with_script(chain, transactions, self.mining_address.clone())
    }

    pub fn mine_block_to(
        &self,
        chain: &ChainState,
        transactions: Vec<Transaction>,
        script_pubkey: Vec<u8>,
    ) -> Result<(BlockHeader, Vec<Transaction>), MiningError> {
        self.mine_block_with_script(chain, transactions, script_pubkey)
    }

    pub fn mine_and_attach(
        &self,
        chain: &mut ChainState,
        transactions: Vec<Transaction>,
    ) -> Result<BlockHeader, MiningError> {
        let (header, txs) = self.mine_block(chain, transactions)?;

        use crate::consensus::block::Block;
        chain
            .add_block(Block {
                header,
                transactions: txs,
            })
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?;

        Ok(header)
    }

    pub fn mine_and_attach_to(
        &self,
        chain: &mut ChainState,
        transactions: Vec<Transaction>,
        script_pubkey: Vec<u8>,
    ) -> Result<BlockHeader, MiningError> {
        let (header, txs) = self.mine_block_to(chain, transactions, script_pubkey)?;

        use crate::consensus::block::Block;
        chain
            .add_block(Block {
                header,
                transactions: txs,
            })
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?;

        Ok(header)
    }
}
