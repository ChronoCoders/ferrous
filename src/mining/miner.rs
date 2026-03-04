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
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_FUTURE_OFFSET: u64 = 7200;

#[derive(Debug, Clone)]
pub struct Miner {
    pub params: ChainParams,
    pub mining_address: Vec<u8>,
    pub event_sender: Option<Sender<MiningEvent>>,
}

#[derive(Debug, Clone)]
pub struct MiningEvent {
    pub height: u32,
    pub nonce: u64,
    pub worker_id: u64,
    pub hash: String,
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
        let header = BlockHeader {
            version: 1,
            prev_block_hash: [0u8; 32],
            merkle_root: [0u8; 32],
            timestamp,
            n_bits,
            nonce: 0,
        };

        let coinbase = Transaction {
            version: 1,
            inputs: vec![TxInput {
                prev_txid: [0u8; 32],
                prev_index: 0xFFFF_FFFF,
                script_sig: vec![0x01],
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

        (header, coinbase)
    }

    fn single_block_chain(genesis_time: u64) -> (ChainState, TempDir) {
        let (genesis, tx) = make_genesis(genesis_time, 0x207f_ffff);
        let temp_dir = TempDir::new().unwrap();
        let chain = ChainState::new(
            crate::consensus::params::Network::Regtest.params(),
            temp_dir.path().to_str().unwrap(),
            Some((genesis, tx)),
        )
        .unwrap();
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
            temp_dir.path().to_str().unwrap(),
            Some((genesis, tx)),
        )
        .unwrap();

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

        let tip = chain.get_tip().unwrap();
        let mut prev_headers = Vec::new();
        let mut current_hash: Hash256 = tip.header.prev_block_hash;

        while let Ok(Some(block)) = chain.get_block(&current_hash) {
            prev_headers.push(block.header);
            if prev_headers.len() == 11 || block.height == 0 {
                break;
            }
            current_hash = block.header.prev_block_hash;
        }

        let res = crate::consensus::validation::validate_timestamp(&tip.header, &prev_headers);
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
        }
    }

    pub fn with_event_sender(mut self, sender: Sender<MiningEvent>) -> Self {
        self.event_sender = Some(sender);
        self
    }

    fn create_coinbase_internal(
        &self,
        height: u32,
        subsidy: u64,
        fees: u64,
        script_pubkey: Vec<u8>,
    ) -> Transaction {
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
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?;
        let height = tip.height + 1;

        let subsidy = crate::consensus::validation::calculate_subsidy(height);
        let total_fees = 0u64;

        let coinbase = self.create_coinbase_internal(height, subsidy, total_fees, script_pubkey);

        let mut all_txs = vec![coinbase];
        all_txs.extend(transactions);

        let txids: Vec<_> = all_txs.iter().map(|tx| tx.txid()).collect();
        let merkle_root = compute_merkle_root(&txids);

        let timestamp = next_block_timestamp(chain)?;

        let target = calculate_next_target(&tip.header, timestamp, &self.params)
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?;

        let n_bits = u256_to_compact(&target);

        let prev_block_hash = tip.header.hash();

        let header = BlockHeader {
            version: 1,
            prev_block_hash,
            merkle_root,
            timestamp,
            n_bits,
            nonce: 0,
        };

        let num_workers = num_cpus::get();
        let found = Arc::new(AtomicBool::new(false));
        let solution_nonce = Arc::new(AtomicU64::new(0));
        let solution_timestamp = Arc::new(AtomicU64::new(timestamp));
        let solution_worker = Arc::new(AtomicU64::new(0));

        let nonce_range = u64::MAX / num_workers as u64;

        (0..num_workers).into_par_iter().for_each(|worker_id| {
            let mut local_header = header;
            let start_nonce = worker_id as u64 * nonce_range;
            let mut current_nonce = start_nonce;
            let mut iterations = 0u64;

            loop {
                if found.load(Ordering::Relaxed) {
                    break;
                }

                if iterations.is_multiple_of(1_000_000) {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    if now > local_header.timestamp {
                        local_header.timestamp = now;
                    }
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

                if current_nonce == start_nonce.wrapping_add(nonce_range) {
                    current_nonce = start_nonce;
                }
            }
        });

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

        chain
            .add_block(header, txs)
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

        chain
            .add_block(header, txs)
            .map_err(|e| MiningError::ChainError(format!("{:?}", e)))?;

        Ok(header)
    }
}
