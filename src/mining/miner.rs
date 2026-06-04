use crate::consensus::block::BlockHeader;
use crate::consensus::chain::ChainState;
use crate::consensus::difficulty::{calculate_next_target, u256_to_compact, DIFFICULTY_WINDOW};
use crate::consensus::merkle::compute_merkle_root;
use crate::consensus::params::ChainParams;
use crate::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
use crate::consensus::validation::MAX_BLOCK_WEIGHT;
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

        let epoch_key = BlockHeader::epoch_key(0);
        while !header.check_proof_of_work(&epoch_key).unwrap() {
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
    #[cfg_attr(not(target_os = "linux"), ignore)]
    fn next_timestamp_nat_less_than_mtp() {
        let nat = 1_000_000_000u64;
        let mtp = nat + 10;

        time::set_mock_time(Some(nat));

        let (chain, _tmp) = single_block_chain(mtp);

        let ts = next_block_timestamp(&chain).expect("timestamp ok");

        assert_eq!(ts, mtp + 1);
    }

    #[test]
    #[cfg_attr(not(target_os = "linux"), ignore)]
    fn next_timestamp_nat_equal_mtp() {
        let nat = 1_000_000_000u64;
        let mtp = nat;

        time::set_mock_time(Some(nat));

        let (chain, _tmp) = single_block_chain(mtp);

        let ts = next_block_timestamp(&chain).expect("timestamp ok");

        assert_eq!(ts, mtp + 1);
    }

    #[test]
    #[cfg_attr(not(target_os = "linux"), ignore)]
    fn next_timestamp_nat_greater_than_mtp() {
        let nat = 1_000_000_000u64;
        let mtp = nat - 10;

        time::set_mock_time(Some(nat));

        let (chain, _tmp) = single_block_chain(mtp);

        let ts = next_block_timestamp(&chain).expect("timestamp ok");

        assert_eq!(ts, nat);
    }

    #[test]
    #[cfg_attr(not(target_os = "linux"), ignore)]
    fn next_timestamp_too_far_in_future() {
        let nat = 1_000_000_000u64;
        let mtp = nat + MAX_FUTURE_OFFSET + 100;

        time::set_mock_time(Some(nat));

        let (chain, _tmp) = single_block_chain(mtp);

        let err = next_block_timestamp(&chain).unwrap_err();

        assert_eq!(err, MiningError::InvalidTimestamp);
    }

    #[test]
    #[cfg_attr(not(target_os = "linux"), ignore)]
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
            let template = miner
                .build_template(&chain, Vec::new())
                .expect("template ok");
            let (header, txs) = miner.solve_template(template).expect("solve ok");
            use crate::consensus::block::Block;
            chain
                .add_block(Block {
                    header,
                    transactions: txs,
                })
                .expect("add_block ok");
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

    fn sync_style_window(chain: &ChainState, prev: &BlockHeader) -> Vec<u64> {
        let mut window = vec![*prev];
        let mut walk_hash = prev.prev_block_hash;
        while window.len() < DIFFICULTY_WINDOW && walk_hash != [0u8; 32] {
            match chain.block_store.get_header(&walk_hash) {
                Ok(Some(ph)) => {
                    walk_hash = ph.prev_block_hash;
                    window.push(ph);
                }
                _ => break,
            }
        }
        window
            .iter()
            .rev()
            .filter(|h| h.prev_block_hash != [0u8; 32])
            .map(|h| h.timestamp)
            .collect()
    }

    #[test]
    #[cfg_attr(not(target_os = "linux"), ignore)]
    fn difficulty_window_identical_across_sites() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        time::set_mock_time(Some(now));

        let (chain, _tmp) = single_block_chain(now.saturating_sub(10));
        let genesis = chain.get_tip().unwrap().unwrap().block.header;
        let genesis_hash = genesis.hash();

        let chain_window = chain.recent_timestamps_ending_at(&genesis_hash, DIFFICULTY_WINDOW);
        assert_eq!(chain_window, Vec::<u64>::new());

        let sync_window = sync_style_window(&chain, &genesis);
        assert_eq!(sync_window, chain_window);

        let next_ts = genesis.timestamp + 1;
        let t1 = calculate_next_target(&genesis, next_ts, &chain.params, &chain_window).unwrap();
        let t2 = calculate_next_target(&genesis, next_ts, &chain.params, &sync_window).unwrap();
        assert_eq!(u256_to_compact(&t1), u256_to_compact(&t2));

        let mut chain = chain;
        let miner = Miner::new(
            crate::consensus::params::Network::Regtest.params(),
            vec![0x51],
        );
        for i in 0..3 {
            time::set_mock_time(Some(now + 1 + i as u64));
            let template = miner
                .build_template(&chain, Vec::new())
                .expect("template ok");
            let (header, txs) = miner.solve_template(template).expect("solve ok");
            use crate::consensus::block::Block;
            chain
                .add_block(Block {
                    header,
                    transactions: txs,
                })
                .expect("add_block ok");
        }

        let tip = chain.get_tip().unwrap().unwrap().block.header;
        let tip_hash = tip.hash();
        let chain_window = chain.recent_timestamps_ending_at(&tip_hash, DIFFICULTY_WINDOW);
        assert_eq!(chain_window.len(), 3);

        let sync_window = sync_style_window(&chain, &tip);
        assert_eq!(sync_window, chain_window);
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

    pub fn build_template_to(
        &self,
        chain: &ChainState,
        transactions: Vec<Transaction>,
        script_pubkey: Vec<u8>,
    ) -> Result<BlockTemplate, MiningError> {
        self.build_template_with_script(chain, transactions, script_pubkey)
    }

    // Returns the block weight contribution of a single transaction.
    fn tx_weight(tx: &Transaction) -> u64 {
        let base = tx.encode_without_witness().len() as u64;
        let total = tx.encode_with_witness().len() as u64;
        base * 3 + total
    }

    // Fee for a candidate tx (Σ input UTXO values − Σ output values), or None
    // if any input is unspendable at `height` (missing UTXO, immature coinbase,
    // or outputs exceed inputs) — such a tx would fail block validation and
    // must not be included in the template.
    fn tx_fee(chain: &ChainState, tx: &Transaction, height: u64) -> Option<u64> {
        use crate::consensus::utxo::OutPoint;
        use crate::consensus::validation::COINBASE_MATURITY;

        let mut input_sum: u64 = 0;
        for input in &tx.inputs {
            let outpoint = OutPoint {
                txid: input.prev_txid,
                vout: input.prev_index,
            };
            let utxo = chain.get_utxo(&outpoint).ok().flatten()?;
            if utxo.coinbase && height < utxo.height.saturating_add(COINBASE_MATURITY) {
                return None;
            }
            input_sum = input_sum.checked_add(utxo.output.value)?;
        }
        let output_sum: u64 = tx.outputs.iter().map(|o| o.value).sum();
        input_sum.checked_sub(output_sum)
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

        let coinbase = self.create_coinbase_internal(height, subsidy, 0, script_pubkey);

        // Reserve 10 % headroom so the solved block never hits MAX_BLOCK_WEIGHT.
        let weight_budget = MAX_BLOCK_WEIGHT * 9 / 10;
        let mut used_weight = Self::tx_weight(&coinbase);

        let mut all_txs = vec![coinbase];
        let mut total_fees: u64 = 0;
        for tx in transactions {
            let w = Self::tx_weight(&tx);
            if used_weight.saturating_add(w) > weight_budget {
                break;
            }
            // Skip txs that would fail validation (stale/missing/immature inputs).
            let fee = match Self::tx_fee(chain, &tx, height as u64) {
                Some(f) => f,
                None => continue,
            };
            total_fees = total_fees.saturating_add(fee);
            used_weight += w;
            all_txs.push(tx);
        }

        // Coinbase claims subsidy + collected fees.  The value field is fixed
        // width, so updating it does not change the weight computed above.
        all_txs[0].outputs[0].value = subsidy + total_fees;

        let txids: Vec<_> = all_txs.iter().map(|tx| tx.txid()).collect();
        let merkle_root = compute_merkle_root(&txids);

        let timestamp = next_block_timestamp(chain)?;

        let prev_block_hash = tip.block.header.hash();
        let window = chain.recent_timestamps_ending_at(&prev_block_hash, DIFFICULTY_WINDOW);
        let target = calculate_next_target(&tip.block.header, timestamp, &self.params, &window)
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?;

        let n_bits = u256_to_compact(&target);

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
        let epoch_key = BlockHeader::epoch_key(height as u64);

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

                if local_header
                    .check_proof_of_work(&epoch_key)
                    .unwrap_or(false)
                {
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
            .check_proof_of_work(&epoch_key)
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?
        {
            return Err(MiningError::ChainError(
                "PoW verification failed".to_string(),
            ));
        }

        Ok((final_header, all_txs))
    }
}
