use ferrous_node::consensus::transaction::{
    Transaction, TxError, TxInput, TxOutput, Witness, MAX_MONEY,
};
use ferrous_node::primitives::hash::{sha256d, Hash256};
use ferrous_node::primitives::serialize::{Decode, DecodeError, Encode};

fn random_hash(byte: u8) -> Hash256 {
    [byte; 32]
}

fn sample_tx_input(script_len: usize, sequence: u32) -> TxInput {
    let mut script = Vec::with_capacity(script_len);
    for i in 0..script_len {
        script.push((i % 256) as u8);
    }

    TxInput {
        prev_txid: random_hash(1),
        prev_index: 0,
        script_sig: script,
        sequence,
    }
}

fn sample_tx_output(value: u64, script_len: usize) -> TxOutput {
    let mut script = Vec::with_capacity(script_len);
    for i in 0..script_len {
        script.push(255u8 - ((i % 256) as u8));
    }

    TxOutput {
        value,
        script_pubkey: script,
    }
}

fn sample_witness(items: &[&[u8]]) -> Witness {
    let mut stack_items = Vec::new();
    for item in items {
        stack_items.push(item.to_vec());
    }
    Witness { stack_items }
}

#[test]
fn test_tx_input_roundtrip() {
    let input = sample_tx_input(10, 0xFFFFFFFF);
    let encoded = input.encode();
    let (decoded, consumed) = TxInput::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded, input);
}

#[test]
fn test_tx_output_roundtrip() {
    let output = sample_tx_output(1_000_000, 20);
    let encoded = output.encode();
    let (decoded, consumed) = TxOutput::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded, output);
}

#[test]
fn test_tx_input_empty_script_sig() {
    let input = sample_tx_input(0, 0);
    let encoded = input.encode();
    let (decoded, consumed) = TxInput::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert!(decoded.script_sig.is_empty());
}

#[test]
fn test_tx_output_empty_script_pubkey() {
    let output = sample_tx_output(5000, 0);
    let encoded = output.encode();
    let (decoded, consumed) = TxOutput::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert!(decoded.script_pubkey.is_empty());
}

#[test]
fn test_tx_input_large_script_1000_bytes() {
    let input = sample_tx_input(1000, 1);
    let encoded = input.encode();
    let (decoded, consumed) = TxInput::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded.script_sig.len(), 1000);
    assert_eq!(decoded, input);
}

#[test]
fn test_tx_output_large_script_pubkey_1000_bytes() {
    let output = sample_tx_output(42, 1000);
    let encoded = output.encode();
    let (decoded, consumed) = TxOutput::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded.script_pubkey.len(), 1000);
    assert_eq!(decoded, output);
}

#[test]
fn test_witness_empty() {
    let witness = Witness {
        stack_items: Vec::new(),
    };
    let encoded = witness.encode();
    let (decoded, consumed) = Witness::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert!(decoded.stack_items.is_empty());
}

#[test]
fn test_witness_single_item() {
    let witness = sample_witness(&[b"item"]);
    let encoded = witness.encode();
    let (decoded, consumed) = Witness::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded, witness);
}

#[test]
fn test_witness_multiple_items() {
    let witness = sample_witness(&[b"item1", b"item2", b"item3"]);
    let encoded = witness.encode();
    let (decoded, consumed) = Witness::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded, witness);
}

#[test]
fn test_witness_roundtrip_large_items() {
    let mut large1 = vec![0u8; 256];
    let mut large2 = vec![0u8; 512];
    for (i, b) in large1.iter_mut().enumerate() {
        *b = (i % 256) as u8;
    }
    for (i, b) in large2.iter_mut().enumerate() {
        *b = 255u8 - ((i % 256) as u8);
    }

    let witness = Witness {
        stack_items: vec![large1.clone(), large2.clone()],
    };

    let encoded = witness.encode();
    let (decoded, consumed) = Witness::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded, witness);
}

#[test]
fn test_has_witness_detection() {
    let tx_without = sample_transaction(false);
    let tx_with = sample_transaction(true);

    assert!(!tx_without.has_witness());
    assert!(tx_with.has_witness());
}

fn sample_transaction(with_witness: bool) -> Transaction {
    let input = sample_tx_input(5, 0xFFFFFFFE);
    let output = sample_tx_output(50_000, 10);

    let witnesses = if with_witness {
        vec![sample_witness(&[b"sig", b"pubkey"])]
    } else {
        vec![Witness {
            stack_items: Vec::new(),
        }]
    };

    Transaction {
        version: 1,
        inputs: vec![input],
        outputs: vec![output],
        witnesses,
        locktime: 0,
    }
}

#[test]
fn test_simple_transaction_no_witness_roundtrip() {
    let tx = sample_transaction(false);
    let encoded = tx.encode_with_witness();
    let (decoded, consumed) = Transaction::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded, tx);
    assert!(!tx.has_witness());
}

#[test]
fn test_transaction_with_witness_roundtrip() {
    let tx = sample_transaction(true);
    let encoded = tx.encode_with_witness();
    let (decoded, consumed) = Transaction::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded, tx);
    assert!(tx.has_witness());
}

#[test]
fn test_transaction_multiple_inputs_outputs_roundtrip() {
    let inputs = vec![
        sample_tx_input(10, 1),
        sample_tx_input(0, 2),
        sample_tx_input(20, 3),
    ];

    let outputs = vec![sample_tx_output(10_000, 5), sample_tx_output(20_000, 6)];

    let witnesses = vec![
        sample_witness(&[b"a"]),
        Witness {
            stack_items: Vec::new(),
        },
        sample_witness(&[b"b", b"c"]),
    ];

    let tx = Transaction {
        version: 2,
        inputs,
        outputs,
        witnesses,
        locktime: 123456,
    };

    let encoded = tx.encode_with_witness();
    let (decoded, consumed) = Transaction::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded, tx);
}

#[test]
fn test_txid_excludes_witness() {
    let tx = sample_transaction(true);

    let without_witness = tx.encode_without_witness();
    let expected_txid = sha256d(&without_witness);

    let with_witness = tx.encode_with_witness();
    let expected_wtxid = sha256d(&with_witness);

    assert_eq!(tx.txid(), expected_txid);
    assert_eq!(tx.wtxid(), expected_wtxid);
    assert_ne!(without_witness.len(), with_witness.len());
}

#[test]
fn test_same_tx_different_witness_same_txid_different_wtxid() {
    let mut tx1 = sample_transaction(true);
    let mut tx2 = sample_transaction(true);

    tx1.witnesses[0] = sample_witness(&[b"sig1"]);
    tx2.witnesses[0] = sample_witness(&[b"sig2"]);

    assert_eq!(tx1.txid(), tx2.txid());
    assert_ne!(tx1.wtxid(), tx2.wtxid());
}

#[test]
fn test_check_structure_valid_transaction() {
    let tx = sample_transaction(true);
    tx.check_structure().expect("structure should be valid");
}

#[test]
fn test_check_structure_no_inputs() {
    let tx = Transaction {
        version: 1,
        inputs: Vec::new(),
        outputs: vec![sample_tx_output(10_000, 5)],
        witnesses: Vec::new(),
        locktime: 0,
    };

    let err = tx.check_structure().unwrap_err();
    assert!(matches!(err, TxError::NoInputs));
}

#[test]
fn test_check_structure_no_outputs() {
    let tx = Transaction {
        version: 1,
        inputs: vec![sample_tx_input(0, 0)],
        outputs: Vec::new(),
        witnesses: vec![Witness {
            stack_items: Vec::new(),
        }],
        locktime: 0,
    };

    let err = tx.check_structure().unwrap_err();
    assert!(matches!(err, TxError::NoOutputs));
}

#[test]
fn test_check_structure_witness_mismatch() {
    let tx = Transaction {
        version: 1,
        inputs: vec![sample_tx_input(0, 0)],
        outputs: vec![sample_tx_output(10_000, 5)],
        witnesses: vec![
            Witness {
                stack_items: Vec::new(),
            },
            Witness {
                stack_items: Vec::new(),
            },
        ],
        locktime: 0,
    };

    let err = tx.check_structure().unwrap_err();
    assert!(matches!(err, TxError::WitnessMismatch));
}

#[test]
fn test_check_structure_value_too_large() {
    let tx = Transaction {
        version: 1,
        inputs: vec![sample_tx_input(0, 0)],
        outputs: vec![sample_tx_output(MAX_MONEY + 1, 0)],
        witnesses: vec![Witness {
            stack_items: Vec::new(),
        }],
        locktime: 0,
    };

    let err = tx.check_structure().unwrap_err();
    assert!(matches!(err, TxError::ValueTooLarge));
}

#[test]
fn test_check_structure_output_sum_overflow() {
    let tx = Transaction {
        version: 1,
        inputs: vec![sample_tx_input(0, 0)],
        outputs: vec![sample_tx_output(u64::MAX, 0), sample_tx_output(u64::MAX, 0)],
        witnesses: vec![Witness {
            stack_items: Vec::new(),
        }],
        locktime: 0,
    };

    let err = tx.check_structure().unwrap_err();
    assert!(matches!(err, TxError::OutputSumOverflow));
}

#[test]
fn test_transaction_max_locktime() {
    let mut tx = sample_transaction(false);
    tx.locktime = u32::MAX;

    let encoded = tx.encode_with_witness();
    let (decoded, consumed) = Transaction::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded.locktime, u32::MAX);
}

#[test]
fn test_tx_input_max_sequence() {
    let input = sample_tx_input(0, u32::MAX);
    let encoded = input.encode();
    let (decoded, consumed) = TxInput::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded.sequence, u32::MAX);
}

#[test]
fn test_transaction_decode_invalid_input_count_varint() {
    let mut bytes = Vec::new();
    let version: u32 = 1;
    bytes.extend_from_slice(&version.encode());
    bytes.push(0xFD);
    bytes.push(0x01);
    bytes.push(0x00);

    let err = Transaction::decode(&bytes).unwrap_err();
    assert!(matches!(err, DecodeError::InvalidData));
}
