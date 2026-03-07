use ferrous_node::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
use ferrous_node::consensus::utxo::{OutPoint, UtxoEntry, UtxoError, UtxoSet};
use ferrous_node::primitives::hash::Hash256;

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

#[test]
fn add_transaction_creates_utxos() {
    let tx = coinbase_transaction(50 * 100_000_000);
    let txid = tx.txid();

    let mut utxos = UtxoSet::new();
    utxos.add_transaction(&tx, 1, true).unwrap();

    let outpoint = OutPoint { txid, vout: 0 };
    let entry = utxos.get(&outpoint).expect("utxo must exist");

    assert_eq!(entry.output.value, 50 * 100_000_000);
    assert_eq!(entry.height, 1);
    assert!(entry.coinbase);
}

#[test]
fn spend_input_removes_utxo() {
    let tx = coinbase_transaction(10_000);
    let txid = tx.txid();

    let mut utxos = UtxoSet::new();
    utxos.add_transaction(&tx, 1, true).unwrap();

    let outpoint = OutPoint { txid, vout: 0 };

    let entry = utxos.spend_input(&outpoint, 200).unwrap();
    assert_eq!(entry.output.value, 10_000);
    assert!(!utxos.contains(&outpoint));
}

#[test]
fn spend_input_missing_utxo_errors() {
    let mut utxos = UtxoSet::new();
    let outpoint = OutPoint {
        txid: zero_hash(),
        vout: 0,
    };

    let result = utxos.spend_input(&outpoint, 1);
    assert_eq!(result, Err(UtxoError::UtxoNotFound));
}

#[test]
fn duplicate_utxo_errors() {
    let tx = coinbase_transaction(10_000);

    let mut utxos = UtxoSet::new();
    utxos.add_transaction(&tx, 1, true).unwrap();

    let result = utxos.add_transaction(&tx, 1, true);
    assert_eq!(result, Err(UtxoError::DuplicateUtxo));
}

#[test]
fn coinbase_not_mature_errors() {
    let tx = coinbase_transaction(10_000);
    let txid = tx.txid();

    let mut utxos = UtxoSet::new();
    utxos.add_transaction(&tx, 1, true).unwrap();

    let outpoint = OutPoint { txid, vout: 0 };

    let result = utxos.spend_input(&outpoint, 50);
    assert_eq!(result, Err(UtxoError::CoinbaseNotMature));
    assert!(utxos.contains(&outpoint));
}

#[test]
fn coinbase_spendable_after_maturity() {
    let tx = coinbase_transaction(10_000);
    let txid = tx.txid();

    let mut utxos = UtxoSet::new();
    utxos.add_transaction(&tx, 1, true).unwrap();

    let outpoint = OutPoint { txid, vout: 0 };

    let entry = utxos.spend_input(&outpoint, 101).unwrap();
    assert_eq!(entry.output.value, 10_000);
    assert!(!utxos.contains(&outpoint));
}

#[test]
fn apply_transaction_spend_and_add() {
    let coinbase = coinbase_transaction(50_000);
    let coinbase_txid = coinbase.txid();

    let mut utxos = UtxoSet::new();
    utxos.add_transaction(&coinbase, 1, true).unwrap();

    let spend_tx = regular_transaction(coinbase_txid, 0, 40_000);
    let spend_txid = spend_tx.txid();

    let spent = utxos.apply_transaction(&spend_tx, 101, false).unwrap();

    assert_eq!(spent.len(), 1);
    assert_eq!(spent[0].output.value, 50_000);

    let spent_outpoint = OutPoint {
        txid: coinbase_txid,
        vout: 0,
    };
    assert!(!utxos.contains(&spent_outpoint));

    let new_outpoint = OutPoint {
        txid: spend_txid,
        vout: 0,
    };
    let new_entry = utxos.get(&new_outpoint).expect("new utxo must exist");
    assert_eq!(new_entry.output.value, 40_000);
    assert!(!new_entry.coinbase);
    assert_eq!(new_entry.height, 101);
}

#[test]
fn get_returns_correct_entry() {
    let tx = coinbase_transaction(7_000);
    let txid = tx.txid();

    let mut utxos = UtxoSet::new();
    utxos.add_transaction(&tx, 5, true).unwrap();

    let outpoint = OutPoint { txid, vout: 0 };
    let entry = utxos.get(&outpoint).unwrap();

    let expected = UtxoEntry {
        output: sample_output(7_000),
        height: 5,
        coinbase: true,
    };

    assert_eq!(*entry, expected);
}

#[test]
fn contains_checks_existence() {
    let tx = coinbase_transaction(3_000);
    let txid = tx.txid();

    let mut utxos = UtxoSet::new();
    utxos.add_transaction(&tx, 2, true).unwrap();

    let existing = OutPoint { txid, vout: 0 };
    let missing = OutPoint { txid, vout: 1 };

    assert!(utxos.contains(&existing));
    assert!(!utxos.contains(&missing));
}

#[test]
fn multiple_outputs_in_one_transaction() {
    let mut tx = coinbase_transaction(0);
    tx.outputs = vec![
        sample_output(1_000),
        sample_output(2_000),
        sample_output(3_000),
    ];
    let txid = tx.txid();

    let mut utxos = UtxoSet::new();
    utxos.add_transaction(&tx, 10, true).unwrap();

    for (index, value) in [1_000u64, 2_000, 3_000].iter().enumerate() {
        let outpoint = OutPoint {
            txid,
            vout: index as u32,
        };
        let entry = utxos.get(&outpoint).expect("utxo must exist");
        assert_eq!(entry.output.value, *value);
        assert_eq!(entry.height, 10);
        assert!(entry.coinbase);
    }
}
