use ferrous_node::consensus::transaction::{Transaction, TxInput, TxOutput};
use ferrous_node::script::sighash::{compute_sighash, SighashError};

fn sample_tx_output(value: u64, script_len: usize) -> TxOutput {
    let mut script = Vec::with_capacity(script_len);
    for i in 0..script_len {
        script.push(255u8 - (i % 256) as u8);
    }
    TxOutput {
        value,
        script_pubkey: script,
    }
}

fn sample_tx_input(index: u32) -> TxInput {
    TxInput {
        prev_txid: [0xAA; 32],
        prev_index: index,
        script_sig: vec![0x01, 0x02],
        sequence: 0xFFFFFFFF,
    }
}

fn sample_transaction(inputs: Vec<TxInput>, outputs: Vec<TxOutput>) -> Transaction {
    Transaction {
        version: 1,
        inputs,
        outputs,
        witnesses: Vec::new(),
        locktime: 0,
    }
}

#[test]
fn test_sighash_basic_structure() {
    let input = sample_tx_input(0);
    let output = sample_tx_output(100_000, 20);
    let tx = sample_transaction(vec![input], vec![output.clone()]);
    let spent_outputs = vec![sample_tx_output(50_000, 25)]; // Different than tx output

    // We can't verify the exact hash value easily without reimplementing the logic,
    // but we can verify it's deterministic and different inputs produce different hashes.
    let hash1 = compute_sighash(&tx, 0, &spent_outputs).unwrap();
    let hash2 = compute_sighash(&tx, 0, &spent_outputs).unwrap();
    assert_eq!(hash1, hash2);
}

#[test]
fn test_sighash_different_inputs_different_hashes() {
    let input = sample_tx_input(0);
    let output = sample_tx_output(100_000, 20);
    let tx = sample_transaction(vec![input], vec![output]);
    let spent_outputs = vec![sample_tx_output(50_000, 25)];

    let hash1 = compute_sighash(&tx, 0, &spent_outputs).unwrap();

    // Modify transaction (change version)
    let mut tx2 = tx.clone();
    tx2.version = 2;
    let hash2 = compute_sighash(&tx2, 0, &spent_outputs).unwrap();
    assert_ne!(hash1, hash2);

    // Modify spent outputs
    let spent_outputs2 = vec![sample_tx_output(50_001, 25)];
    let hash3 = compute_sighash(&tx, 0, &spent_outputs2).unwrap();
    assert_ne!(hash1, hash3);
}

#[test]
fn test_sighash_multiple_inputs() {
    let inputs = vec![sample_tx_input(0), sample_tx_input(1)];
    let output = sample_tx_output(100_000, 20);
    let tx = sample_transaction(inputs, vec![output]);
    let spent_outputs = vec![sample_tx_output(50_000, 25), sample_tx_output(40_000, 25)];

    // Compute for input 0
    let hash0 = compute_sighash(&tx, 0, &spent_outputs).unwrap();

    // Compute for input 1
    let hash1 = compute_sighash(&tx, 1, &spent_outputs).unwrap();

    // Should be different because input_index is part of the sighash
    assert_ne!(hash0, hash1);
}

#[test]
fn test_sighash_input_index_out_of_bounds() {
    let input = sample_tx_input(0);
    let output = sample_tx_output(100_000, 20);
    let tx = sample_transaction(vec![input], vec![output]);
    let spent_outputs = vec![sample_tx_output(50_000, 25)];

    let err = compute_sighash(&tx, 1, &spent_outputs).unwrap_err();
    assert_eq!(err, SighashError::InputIndexOutOfBounds);
}

#[test]
fn test_sighash_spent_outputs_mismatch() {
    let input = sample_tx_input(0);
    let output = sample_tx_output(100_000, 20);
    let tx = sample_transaction(vec![input], vec![output]);
    let spent_outputs = vec![sample_tx_output(50_000, 25), sample_tx_output(40_000, 25)];

    let err = compute_sighash(&tx, 0, &spent_outputs).unwrap_err();
    assert_eq!(err, SighashError::SpentOutputsMismatch);
}

#[test]
fn test_component_hashes_consistency() {
    // This tests that modifying any component changes the final sighash
    let input = sample_tx_input(0);
    let output = sample_tx_output(100_000, 20);
    let tx = sample_transaction(vec![input], vec![output]);
    let spent_outputs = vec![sample_tx_output(50_000, 25)];

    let base_hash = compute_sighash(&tx, 0, &spent_outputs).unwrap();

    // 1. Modify input sequence
    let mut tx_seq = tx.clone();
    tx_seq.inputs[0].sequence = 0;
    assert_ne!(
        base_hash,
        compute_sighash(&tx_seq, 0, &spent_outputs).unwrap()
    );

    // 2. Modify input prev_txid
    let mut tx_prev = tx.clone();
    tx_prev.inputs[0].prev_txid = [0xBB; 32];
    assert_ne!(
        base_hash,
        compute_sighash(&tx_prev, 0, &spent_outputs).unwrap()
    );

    // 3. Modify output amount
    let mut tx_out = tx.clone();
    tx_out.outputs[0].value += 1;
    assert_ne!(
        base_hash,
        compute_sighash(&tx_out, 0, &spent_outputs).unwrap()
    );

    // 4. Modify locktime
    let mut tx_lock = tx.clone();
    tx_lock.locktime = 100;
    assert_ne!(
        base_hash,
        compute_sighash(&tx_lock, 0, &spent_outputs).unwrap()
    );
}
