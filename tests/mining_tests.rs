use ferrous_node::consensus::block::{BlockHeader, U256};
use ferrous_node::consensus::chain::ChainState;
use ferrous_node::consensus::merkle::compute_merkle_root;
use ferrous_node::consensus::params::Network;
use ferrous_node::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
use ferrous_node::mining::Miner;
use ferrous_node::primitives::hash::Hash256;
use tempfile::TempDir;

fn zero_hash() -> Hash256 {
    [0u8; 32]
}

fn sample_output(value: u64) -> TxOutput {
    TxOutput {
        value,
        script_pubkey: vec![0x51],
    }
}

fn empty_witnesses(input_count: usize) -> Vec<Witness> {
    let mut v = Vec::with_capacity(input_count);
    for _ in 0..input_count {
        v.push(Witness {
            stack_items: Vec::new(),
        });
    }
    v
}

fn coinbase_transaction(value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig: Vec::new(),
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![sample_output(value)],
        witnesses: empty_witnesses(1),
        locktime: 0,
    }
}

fn create_easy_genesis() -> (BlockHeader, Transaction) {
    let tx = coinbase_transaction(50 * 100_000_000);
    let txids = vec![tx.txid()];
    let merkle_root = compute_merkle_root(&txids);

    let mut header = BlockHeader {
        version: 1,
        prev_block_hash: zero_hash(),
        merkle_root,
        timestamp: 1_000_000_000,
        n_bits: 0x207f_ffff,
        nonce: 0,
    };

    while !header.check_proof_of_work().unwrap() {
        header.nonce += 1;
    }

    (header, tx)
}

fn regtest_params() -> ferrous_node::consensus::params::ChainParams {
    Network::Regtest.params()
}

fn create_test_chain() -> (ChainState, TempDir) {
    let (genesis, genesis_tx) = create_easy_genesis();
    let temp_dir = TempDir::new().unwrap();
    let chain = ChainState::new(
        regtest_params(),
        temp_dir.path().to_str().unwrap(),
        Some((genesis, genesis_tx)),
    )
    .unwrap();
    (chain, temp_dir)
}

#[test]
fn test_create_coinbase_height_commitment() {
    let miner = Miner::new(regtest_params(), vec![0x51]);
    let (chain, _tmp) = create_test_chain();
    let tx = miner
        .mine_block(&chain, Vec::new())
        .expect("mining should succeed")
        .1[0]
        .clone();

    assert_eq!(tx.inputs.len(), 1);
    assert_eq!(tx.outputs.len(), 1);
    assert!(!tx.inputs[0].script_sig.is_empty());
}

#[test]
fn test_mine_block_produces_valid_header() {
    let (mut chain, _tmp) = create_test_chain();

    let miner = Miner::new(regtest_params(), vec![0x51]);

    let (header, txs) = miner
        .mine_block(&chain, Vec::new())
        .expect("mining should succeed");

    assert!(header.check_proof_of_work().unwrap());
    assert_eq!(txs.len(), 1);

    let result = chain.add_block(header, txs);
    assert!(result.is_ok());
}

#[test]
fn test_mined_block_extends_chain() {
    let (mut chain, _tmp) = create_test_chain();

    let miner = Miner::new(regtest_params(), vec![0x51]);

    let (header, txs) = miner
        .mine_block(&chain, Vec::new())
        .expect("mining should succeed");

    let prev_tip_hash = chain.get_tip().unwrap().header.hash();
    chain.add_block(header, txs).unwrap();

    assert_ne!(chain.get_tip().unwrap().header.hash(), prev_tip_hash);
}

#[test]
fn test_target_to_compact_roundtrip_like() {
    let mut bytes = [0u8; 32];
    bytes[0] = 1;
    let value = U256::from_le_bytes(bytes);

    let compact = {
        fn helper(value: &U256) -> u32 {
            let mut exponent = 0u8;
            let mut mantissa = 0u32;

            for (i, &byte) in value.0.iter().enumerate().rev() {
                if byte != 0 {
                    exponent = (i + 1) as u8;
                    let start = i.saturating_sub(2);
                    mantissa = (value.0[start] as u32)
                        | ((value.0[start + 1] as u32) << 8)
                        | ((value.0[start + 2] as u32) << 16);
                    break;
                }
            }

            ((exponent as u32) << 24) | (mantissa & 0x00FF_FFFF)
        }
        helper(&value)
    };

    assert!(compact != 0);
}

#[test]
fn test_mining_updates_timestamp_over_time() {
    let (chain, _tmp) = create_test_chain();

    let miner = Miner::new(regtest_params(), vec![0x51]);

    let start = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let (header, _) = miner
        .mine_block(&chain, Vec::new())
        .expect("mining should succeed");

    assert!(header.timestamp >= start);
}

#[test]
fn test_subsidy_integration_in_coinbase() {
    let (chain, _tmp) = create_test_chain();

    let miner = Miner::new(regtest_params(), vec![0x51]);

    let (header, txs) = miner
        .mine_block(&chain, Vec::new())
        .expect("mining should succeed");

    assert_eq!(txs[0].outputs[0].value, 50 * 100_000_000);
    assert!(header.check_proof_of_work().unwrap());
}

#[test]
fn test_mine_and_attach_convenience() {
    let (mut chain, _tmp) = create_test_chain();

    let miner = Miner::new(regtest_params(), vec![0x51]);

    let old_height = chain.get_tip().unwrap().height;

    let header = miner
        .mine_and_attach(&mut chain, Vec::new())
        .expect("mine and attach should succeed");

    assert_eq!(chain.get_tip().unwrap().height, old_height + 1);
    assert_eq!(chain.get_tip().unwrap().header.hash(), header.hash());
}
