use ferrous_node::consensus::block::BlockHeader;
use ferrous_node::consensus::chain::{ChainError, ChainState};
use ferrous_node::consensus::merkle::compute_merkle_root;
use ferrous_node::consensus::params::Network;
use ferrous_node::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
use ferrous_node::consensus::utxo::{OutPoint, UtxoError};
use ferrous_node::primitives::hash::{sha256d, Hash256};
use ferrous_node::script::engine::{validate_p2pkh, ScriptContext};
use ferrous_node::wallet::address::address_to_script_pubkey;
use ferrous_node::wallet::builder::TransactionBuilder;
use ferrous_node::wallet::manager::Wallet;
use std::collections::HashSet;
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

fn coinbase_transaction(value: u64, extra: u8) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig: vec![extra],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![sample_output(value)],
        witnesses: empty_witnesses(1),
        locktime: 0,
    }
}

fn regular_transaction(prev_txid: Hash256, prev_index: u32, output_value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid,
            prev_index,
            script_sig: vec![0x51],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![sample_output(output_value)],
        witnesses: empty_witnesses(1),
        locktime: 0,
    }
}

fn create_genesis_block() -> (BlockHeader, Transaction) {
    let tx = coinbase_transaction(50 * 100_000_000, 0);
    let txids = vec![tx.txid()];
    let merkle_root = compute_merkle_root(&txids);

    let header = BlockHeader {
        version: 1,
        prev_block_hash: zero_hash(),
        merkle_root,
        timestamp: 1_000_000_000,
        n_bits: 0x207f_ffff,
        nonce: 0,
    };

    // We don't mine it here, but we assume it's valid for genesis
    (header, tx)
}

fn mine_block(
    prev_header: &BlockHeader,
    transactions: &[Transaction],
    timestamp: u64,
) -> BlockHeader {
    let txids: Vec<_> = transactions.iter().map(|tx| tx.txid()).collect();
    let merkle_root = compute_merkle_root(&txids);

    let mut header = BlockHeader {
        version: 1,
        prev_block_hash: prev_header.hash(),
        merkle_root,
        timestamp,
        n_bits: 0x207f_ffff, // Easy target for tests
        nonce: 0,
    };

    // Simple mining loop
    while !header.check_proof_of_work().unwrap() {
        header.nonce += 1;
    }

    header
}

fn create_test_chain() -> (ChainState, TempDir) {
    let (genesis, genesis_tx) = create_genesis_block();
    let temp_dir = TempDir::new().unwrap();
    let chain = ChainState::new(
        Network::Regtest.params(),
        temp_dir.path().to_str().unwrap(),
        Some((genesis, genesis_tx)),
    )
    .unwrap();
    (chain, temp_dir)
}

#[test]
fn test_genesis_initialization() {
    let (chain, _tmp) = create_test_chain();
    let (genesis, _) = create_genesis_block();

    assert_eq!(chain.get_tip().unwrap().height, 0);
    assert_eq!(chain.get_tip().unwrap().header.hash(), genesis.hash());
}

#[test]
fn test_add_valid_block_extends_tip() {
    let (mut chain, _tmp) = create_test_chain();

    let prev_header = chain.get_tip().unwrap().header;
    let tx = coinbase_transaction(50 * 100_000_000, 1);
    let header = mine_block(
        &prev_header,
        std::slice::from_ref(&tx),
        prev_header.timestamp + 600,
    );

    let result = chain.add_block(header, vec![tx]);
    assert_eq!(result, Ok(true));

    assert_eq!(chain.get_tip().unwrap().height, 1);
    assert_eq!(chain.get_tip().unwrap().header.hash(), header.hash());
}

#[test]
fn test_add_orphan_block_error() {
    let (mut chain, _tmp) = create_test_chain();

    let prev_header = chain.get_tip().unwrap().header;
    let tx = coinbase_transaction(50 * 100_000_000, 1);

    // Create block that points to random parent
    let mut header = mine_block(
        &prev_header,
        std::slice::from_ref(&tx),
        prev_header.timestamp + 600,
    );
    header.prev_block_hash = sha256d(&[1u8; 32]);

    let result = chain.add_block(header, vec![tx]);
    assert_eq!(result, Err(ChainError::OrphanBlock));
}

#[test]
fn test_reorg_to_longer_chain() {
    let (mut chain, _tmp) = create_test_chain();

    let genesis_header = chain.get_tip().unwrap().header;

    // Chain A: Genesis -> A1
    let tx_a1 = coinbase_transaction(50 * 100_000_000, 1);
    let header_a1 = mine_block(
        &genesis_header,
        std::slice::from_ref(&tx_a1),
        genesis_header.timestamp + 600,
    );
    chain.add_block(header_a1, vec![tx_a1]).unwrap();

    assert_eq!(chain.get_tip().unwrap().header.hash(), header_a1.hash());

    // Chain B: Genesis -> B1 -> B2 (Longer/Heavier)
    // Note: Since difficulty is same, length 2 > length 1 means more work.

    let tx_b1 = coinbase_transaction(50 * 100_000_000, 2);
    // Use slightly different timestamp to ensure different hash from A1
    let header_b1 = mine_block(
        &genesis_header,
        std::slice::from_ref(&tx_b1),
        genesis_header.timestamp + 601,
    );

    // Add B1 (side chain, not tip yet)
    let result = chain.add_block(header_b1, vec![tx_b1]);
    assert_eq!(result, Ok(false)); // Valid but not tip
    assert_eq!(chain.get_tip().unwrap().header.hash(), header_a1.hash());

    // Add B2
    let tx_b2 = coinbase_transaction(50 * 100_000_000, 3);
    let header_b2 = mine_block(
        &header_b1,
        std::slice::from_ref(&tx_b2),
        header_b1.timestamp + 600,
    );

    let result = chain.add_block(header_b2, vec![tx_b2]);
    assert_eq!(result, Ok(true)); // Reorg! New tip

    assert_eq!(chain.get_tip().unwrap().header.hash(), header_b2.hash());
    assert_eq!(chain.get_tip().unwrap().height, 2);
}

#[test]
fn test_block_valid_but_not_tip() {
    let (mut chain, _tmp) = create_test_chain();

    let genesis_header = chain.get_tip().unwrap().header;

    // Tip: Genesis -> A1
    let tx_a1 = coinbase_transaction(50 * 100_000_000, 1);
    let header_a1 = mine_block(
        &genesis_header,
        std::slice::from_ref(&tx_a1),
        genesis_header.timestamp + 600,
    );
    chain.add_block(header_a1, vec![tx_a1]).unwrap();

    // Side: Genesis -> B1 (Same work, but arrived later, so not tip)
    let tx_b1 = coinbase_transaction(50 * 100_000_000, 2);
    let header_b1 = mine_block(
        &genesis_header,
        std::slice::from_ref(&tx_b1),
        genesis_header.timestamp + 601,
    );

    let result = chain.add_block(header_b1, vec![tx_b1]);
    assert_eq!(result, Ok(false));
    assert_eq!(chain.get_tip().unwrap().header.hash(), header_a1.hash());
}

#[test]
fn test_get_block() {
    let (mut chain, _tmp) = create_test_chain();

    let genesis_header = chain.get_tip().unwrap().header;

    let tx = coinbase_transaction(50 * 100_000_000, 1);
    let header = mine_block(
        &genesis_header,
        std::slice::from_ref(&tx),
        genesis_header.timestamp + 600,
    );
    chain.add_block(header, vec![tx]).unwrap();

    assert!(chain.get_block(&header.hash()).unwrap().is_some());
    assert!(chain.get_block(&sha256d(&[0u8; 32])).unwrap().is_none());
}

#[test]
fn test_cumulative_work_calculation() {
    let (mut chain, _tmp) = create_test_chain();

    let work_genesis = chain.get_tip().unwrap().cumulative_work;
    assert!(work_genesis > 0);

    let genesis_header = chain.get_tip().unwrap().header;
    let tx = coinbase_transaction(50 * 100_000_000, 1);
    let header = mine_block(
        &genesis_header,
        std::slice::from_ref(&tx),
        genesis_header.timestamp + 600,
    );
    chain.add_block(header, vec![tx]).unwrap();

    let work_tip = chain.get_tip().unwrap().cumulative_work;
    assert!(work_tip > work_genesis);
}

#[test]
fn test_deep_reorg_20_blocks() {
    let (mut chain, _tmp) = create_test_chain();

    let genesis_header = chain.get_tip().unwrap().header;

    // Main chain A: 20 blocks on top of genesis
    let mut prev_header = genesis_header;
    for i in 0..20 {
        let tx = coinbase_transaction(50 * 100_000_000, (i + 1) as u8);
        let header = mine_block(
            &prev_header,
            std::slice::from_ref(&tx),
            prev_header.timestamp + 600,
        );
        chain.add_block(header, vec![tx]).unwrap();
        prev_header = header;
    }

    let tip_a = chain.get_tip().unwrap();
    assert_eq!(tip_a.height, 20);

    // Competing chain B from genesis: 25 blocks, longer/heavier
    let (genesis_header_again, _) = create_genesis_block();
    let mut prev_b = genesis_header_again;
    let mut b_tip_hash = Hash256::default();

    for i in 0..25 {
        let tx = coinbase_transaction(50 * 100_000_000, (i + 50) as u8);
        let header = mine_block(&prev_b, std::slice::from_ref(&tx), prev_b.timestamp + 600);
        b_tip_hash = header.hash();
        chain.add_block(header, vec![tx]).unwrap();
        prev_b = header;
    }

    let tip_b = chain.get_tip().unwrap();
    assert_eq!(tip_b.height, 25);
    assert_eq!(tip_b.header.hash(), b_tip_hash);

    // UTXO set must contain only coinbases from genesis + B chain
    let utxos1 = chain.export_utxos().unwrap();
    assert!(!utxos1.is_empty());

    let mut heights = HashSet::new();
    for (_, entry) in &utxos1 {
        assert!(entry.is_coinbase);
        heights.insert(entry.height);
    }

    // Expect exactly one UTXO per height 0..=25
    assert_eq!(heights.len(), 26);
    assert!(heights.contains(&0));
    assert!(heights.contains(&25));

    // Deterministic export: repeated calls must match
    let utxos2 = chain.export_utxos().unwrap();
    assert_eq!(utxos1, utxos2);
}

#[test]
fn test_wallet_balance_after_reorg() {
    let (mut chain, _tmp_chain) = create_test_chain();

    // Create temporary wallet
    let wallet_dir = TempDir::new().unwrap();
    let wallet_path = wallet_dir.path().join("wallet.dat");
    let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();
    let address = wallet.generate_address().unwrap();
    let script = address_to_script_pubkey(&address).unwrap();

    let genesis_header = chain.get_tip().unwrap().header;

    // Main chain A: height 105, with a single wallet-owned coinbase at height 1
    let mut prev_header = genesis_header;

    // Height 1: coinbase to wallet script
    let mut tx_wallet = coinbase_transaction(50 * 100_000_000, 1);
    tx_wallet.outputs[0].script_pubkey = script.clone();
    let header_wallet = mine_block(
        &prev_header,
        std::slice::from_ref(&tx_wallet),
        prev_header.timestamp + 600,
    );
    chain.add_block(header_wallet, vec![tx_wallet]).unwrap();
    prev_header = header_wallet;

    // Heights 2..105: regular coinbases to non-wallet script
    for i in 2..=105 {
        let tx = coinbase_transaction(50 * 100_000_000, i as u8);
        let header = mine_block(
            &prev_header,
            std::slice::from_ref(&tx),
            prev_header.timestamp + 600,
        );
        chain.add_block(header, vec![tx]).unwrap();
        prev_header = header;
    }

    assert_eq!(chain.get_tip().unwrap().height, 105);

    // Wallet should see one matured coinbase output
    let balance_before = wallet.get_balance(&chain).unwrap();
    assert_eq!(balance_before, 50 * 100_000_000);

    // Competing chain B: 110 blocks from genesis, none paying to the wallet
    let (genesis_header_again, _) = create_genesis_block();
    let mut prev_b = genesis_header_again;

    for i in 0..110 {
        let tx = coinbase_transaction(50 * 100_000_000, (i + 1) as u8);
        let header = mine_block(&prev_b, std::slice::from_ref(&tx), prev_b.timestamp + 600);
        let res = chain.add_block(header, vec![tx]);
        assert!(res.is_ok(), "reorg add_block failed at i={i}: {:?}", res);
        prev_b = header;
    }

    let tip = chain.get_tip().unwrap();
    assert_eq!(tip.height, 110);

    // After reorg, wallet's previous coinbase no longer exists in main chain
    let balance_after = wallet.get_balance(&chain).unwrap();
    assert_eq!(balance_after, 0);
}

#[test]
fn test_invalid_difficulty_error() {
    let (mut chain, _tmp) = create_test_chain();

    let prev_header = chain.get_tip().unwrap().header;
    let tx = coinbase_transaction(50 * 100_000_000, 1);

    // Create header with wrong n_bits
    let mut header = mine_block(
        &prev_header,
        std::slice::from_ref(&tx),
        prev_header.timestamp + 600,
    );
    header.n_bits = 0x207f_fffe; // Slightly different
                                 // Remine to make PoW valid for this n_bits
    while !header.check_proof_of_work().unwrap() {
        header.nonce += 1;
    }

    // But validation against parent should fail because it expects next target to be same (or similar)
    // Note: n_bits change is small so it might pass EMA validation if it's within bounds?
    // Wait, EMA difficulty adjusts target. If we force n_bits to something else, it might fail validation
    // if it doesn't match the calculated target.
    // In our case, time delta is 600 (max).
    // Let's use a very wrong n_bits
    header.n_bits = 0x2000_ffff; // Way easier

    while !header.check_proof_of_work().unwrap() {
        header.nonce += 1;
    }

    let result = chain.add_block(header, vec![tx]);
    // Should fail difficulty validation
    assert!(matches!(result, Err(ChainError::InvalidDifficulty(_))));
}

#[test]
fn test_double_spend_prevention() {
    let (mut chain, _tmp) = create_test_chain();

    let genesis_header = chain.get_tip().unwrap().header;

    let mut prev_header = genesis_header;
    let mut coinbase_to_spend_txid = [0u8; 32];

    for height in 1..=120 {
        if height == 1 {
            let tx = coinbase_transaction(50 * 100_000_000, 1);
            coinbase_to_spend_txid = tx.txid();
            let header = mine_block(
                &prev_header,
                std::slice::from_ref(&tx),
                prev_header.timestamp + 600,
            );
            chain.add_block(header, vec![tx]).unwrap();
            prev_header = header;
        } else if height == 120 {
            let coinbase_tx = coinbase_transaction(50 * 100_000_000, 2);
            let spend_tx = regular_transaction(coinbase_to_spend_txid, 0, 40_000);
            let txs = vec![coinbase_tx, spend_tx];
            let header = mine_block(&prev_header, &txs, prev_header.timestamp + 600);
            let added = chain.add_block(header, txs).unwrap();
            assert!(added);
            prev_header = header;
        } else {
            let tx = coinbase_transaction(50 * 100_000_000, (height as u8).wrapping_add(10));
            let header = mine_block(
                &prev_header,
                std::slice::from_ref(&tx),
                prev_header.timestamp + 600,
            );
            chain.add_block(header, vec![tx]).unwrap();
            prev_header = header;
        }
    }

    assert_eq!(chain.get_tip().unwrap().height, 120);

    let original_outpoint = OutPoint {
        txid: coinbase_to_spend_txid,
        index: 0,
    };

    let utxos = chain.export_utxos().unwrap();
    assert!(!utxos.iter().any(|(op, _)| *op == original_outpoint));

    let coinbase_tx2 = coinbase_transaction(50 * 100_000_000, 3);
    let double_spend_tx = regular_transaction(coinbase_to_spend_txid, 0, 30_000);
    let txs2 = vec![coinbase_tx2, double_spend_tx];
    let header2 = mine_block(&prev_header, &txs2, prev_header.timestamp + 600);

    let result = chain.add_block(header2, txs2);
    assert!(matches!(
        result,
        Err(ChainError::UtxoError(UtxoError::UtxoNotFound))
    ));

    assert_eq!(chain.get_tip().unwrap().height, 120);
}

#[test]
fn test_transaction_signing_correctness() {
    let (mut chain, _tmp_chain) = create_test_chain();

    let wallet_dir = TempDir::new().unwrap();
    let wallet_path = wallet_dir.path().join("wallet.dat");
    let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();

    let addr1 = wallet.generate_address().unwrap();
    let addr2 = wallet.generate_address().unwrap();
    let addr_dest = wallet.generate_address().unwrap();

    let script1 = address_to_script_pubkey(&addr1).unwrap();
    let script2 = address_to_script_pubkey(&addr2).unwrap();

    let genesis_header = chain.get_tip().unwrap().header;

    let mut prev_header = genesis_header;

    let mut tx1 = coinbase_transaction(50 * 100_000_000, 1);
    tx1.outputs[0].script_pubkey = script1;
    let header1 = mine_block(
        &prev_header,
        std::slice::from_ref(&tx1),
        prev_header.timestamp + 600,
    );
    chain.add_block(header1, vec![tx1]).unwrap();
    prev_header = header1;

    let mut tx2 = coinbase_transaction(50 * 100_000_000, 2);
    tx2.outputs[0].script_pubkey = script2;
    let header2 = mine_block(
        &prev_header,
        std::slice::from_ref(&tx2),
        prev_header.timestamp + 600,
    );
    chain.add_block(header2, vec![tx2]).unwrap();
    prev_header = header2;

    for i in 3..=105 {
        let tx = coinbase_transaction(50 * 100_000_000, i as u8);
        let header = mine_block(
            &prev_header,
            std::slice::from_ref(&tx),
            prev_header.timestamp + 600,
        );
        chain.add_block(header, vec![tx]).unwrap();
        prev_header = header;
    }

    let balance = wallet.get_balance(&chain).unwrap();
    assert_eq!(balance, 100 * 100_000_000);

    let tx = TransactionBuilder::create_transaction(
        &wallet,
        &chain,
        &addr_dest,
        70 * 100_000_000,
        1_000,
    )
    .unwrap();

    assert!(tx.inputs.len() >= 2);

    let utxos = chain.export_utxos().unwrap();
    let mut spent_outputs = Vec::new();

    for input in &tx.inputs {
        let outpoint = OutPoint {
            txid: input.prev_txid,
            index: input.prev_index,
        };
        let output = utxos
            .iter()
            .find(|(op, _)| *op == outpoint)
            .map(|(_, entry)| entry.output.clone())
            .unwrap();
        spent_outputs.push(output);
    }

    for (index, input) in tx.inputs.iter().enumerate() {
        let script_pubkey = &spent_outputs[index].script_pubkey;
        let context = ScriptContext {
            transaction: &tx,
            input_index: index,
            spent_outputs: &spent_outputs,
        };
        let result = validate_p2pkh(&input.script_sig, script_pubkey, &context).unwrap();
        assert!(result);
    }
}

#[test]
fn test_invalid_signature_rejection() {
    let (mut chain, _tmp_chain) = create_test_chain();

    let wallet_dir = TempDir::new().unwrap();
    let wallet_path = wallet_dir.path().join("wallet.dat");
    let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();
    let addr = wallet.generate_address().unwrap();
    let dest = wallet.generate_address().unwrap();
    let script = address_to_script_pubkey(&addr).unwrap();

    let genesis_header = chain.get_tip().unwrap().header;
    let mut prev_header = genesis_header;

    let mut tx_coinbase = coinbase_transaction(50 * 100_000_000, 1);
    tx_coinbase.outputs[0].script_pubkey = script;
    let header1 = mine_block(
        &prev_header,
        std::slice::from_ref(&tx_coinbase),
        prev_header.timestamp + 600,
    );
    chain.add_block(header1, vec![tx_coinbase]).unwrap();
    prev_header = header1;

    for i in 2..=105 {
        let tx = coinbase_transaction(50 * 100_000_000, i as u8);
        let header = mine_block(
            &prev_header,
            std::slice::from_ref(&tx),
            prev_header.timestamp + 600,
        );
        chain.add_block(header, vec![tx]).unwrap();
        prev_header = header;
    }

    let tx_signed =
        TransactionBuilder::create_transaction(&wallet, &chain, &dest, 10 * 100_000_000, 1_000)
            .unwrap();

    let mut tx_invalid = tx_signed.clone();
    tx_invalid.inputs[0].script_sig[1] ^= 0xff;

    let coinbase = coinbase_transaction(50 * 100_000_000, 200);
    let txs = vec![coinbase, tx_invalid];
    let header = mine_block(&prev_header, &txs, prev_header.timestamp + 600);

    let result = chain.add_block(header, txs);
    assert!(matches!(
        result,
        Err(ChainError::UtxoError(UtxoError::ScriptValidationFailed))
    ));

    assert_eq!(chain.get_tip().unwrap().height, 105);
}
