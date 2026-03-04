use ferrous_node::consensus::block::BlockHeader;
use ferrous_node::consensus::merkle::{compute_merkle_root, compute_witness_merkle_root};
use ferrous_node::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
use ferrous_node::consensus::validation::{
    calculate_subsidy, validate_block, validate_coinbase_height, validate_coinbase_reward,
    validate_timestamp, validate_witness_commitment, ValidationError,
};
use ferrous_node::primitives::hash::{sha256d, Hash256};

fn random_hash(byte: u8) -> Hash256 {
    [byte; 32]
}

fn sample_coinbase(value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig: Vec::new(),
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value,
            script_pubkey: Vec::new(),
        }],
        witnesses: vec![Witness {
            stack_items: Vec::new(),
        }],
        locktime: 0,
    }
}

fn sample_transaction(prev_txid_byte: u8, value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: random_hash(prev_txid_byte),
            prev_index: 0,
            script_sig: vec![0x51],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value,
            script_pubkey: vec![0x51],
        }],
        witnesses: vec![Witness {
            stack_items: Vec::new(),
        }],
        locktime: 0,
    }
}

fn header_with_valid_pow(transactions: &[Transaction]) -> BlockHeader {
    let txids: Vec<_> = transactions.iter().map(|tx| tx.txid()).collect();
    let merkle_root = compute_merkle_root(&txids);

    let mut header = BlockHeader {
        version: 1,
        prev_block_hash: random_hash(1),
        merkle_root,
        timestamp: 1_700_000_000,
        n_bits: 0x207f_ffff,
        nonce: 0,
    };

    for nonce in 0u32..u32::MAX {
        header.nonce = u64::from(nonce);
        if header.check_proof_of_work().unwrap_or(false) {
            return header;
        }
    }

    panic!("failed to find valid proof-of-work for test header");
}

#[test]
fn test_valid_block_passes_all_checks() {
    let coinbase = sample_coinbase(50 * 100_000_000);
    let tx1 = sample_transaction(1, 10_000);
    let tx2 = sample_transaction(2, 20_000);
    let transactions = vec![coinbase, tx1, tx2];

    let header = header_with_valid_pow(&transactions);

    let result = validate_block(&header, &transactions);
    assert_eq!(result, Ok(()));
}

#[test]
fn test_no_transactions_error() {
    let empty: Vec<Transaction> = Vec::new();

    let header = BlockHeader {
        version: 1,
        prev_block_hash: random_hash(0),
        merkle_root: random_hash(1),
        timestamp: 1_700_000_000,
        n_bits: 0x1d00_ffff,
        nonce: 0,
    };

    let result = validate_block(&header, &empty);
    assert_eq!(result, Err(ValidationError::NoTransactions));
}

#[test]
fn test_no_coinbase_error() {
    let tx1 = sample_transaction(1, 10_000);
    let tx2 = sample_transaction(2, 20_000);
    let transactions = vec![tx1.clone(), tx2];

    let txids: Vec<_> = transactions.iter().map(|tx| tx.txid()).collect();
    let merkle_root = compute_merkle_root(&txids);

    let header = BlockHeader {
        version: 1,
        prev_block_hash: random_hash(0),
        merkle_root,
        timestamp: 1_700_000_000,
        n_bits: 0x1d00_ffff,
        nonce: 0,
    };

    let result = validate_block(&header, &transactions);
    assert_eq!(result, Err(ValidationError::NoCoinbase));
}

#[test]
fn test_multiple_coinbases_error() {
    let coinbase1 = sample_coinbase(50 * 100_000_000);
    let coinbase2 = sample_coinbase(25 * 100_000_000);
    let tx = sample_transaction(1, 10_000);
    let transactions = vec![coinbase1, coinbase2, tx];

    let txids: Vec<_> = transactions.iter().map(|tx| tx.txid()).collect();
    let merkle_root = compute_merkle_root(&txids);

    let header = BlockHeader {
        version: 1,
        prev_block_hash: random_hash(0),
        merkle_root,
        timestamp: 1_700_000_000,
        n_bits: 0x1d00_ffff,
        nonce: 0,
    };

    let result = validate_block(&header, &transactions);
    assert_eq!(result, Err(ValidationError::MultipleCoinbases));
}

#[test]
fn test_invalid_merkle_root_error() {
    let coinbase = sample_coinbase(50 * 100_000_000);
    let tx1 = sample_transaction(1, 10_000);
    let transactions = vec![coinbase, tx1];

    let header = BlockHeader {
        version: 1,
        prev_block_hash: random_hash(0),
        merkle_root: random_hash(9),
        timestamp: 1_700_000_000,
        n_bits: 0x1d00_ffff,
        nonce: 0,
    };

    let result = validate_block(&header, &transactions);
    assert_eq!(result, Err(ValidationError::InvalidMerkleRoot));
}

#[test]
fn test_invalid_proof_of_work_error() {
    let coinbase = sample_coinbase(50 * 100_000_000);
    let tx1 = sample_transaction(1, 10_000);
    let transactions = vec![coinbase, tx1];

    let txids: Vec<_> = transactions.iter().map(|tx| tx.txid()).collect();
    let merkle_root = compute_merkle_root(&txids);

    let header = BlockHeader {
        version: 1,
        prev_block_hash: random_hash(0),
        merkle_root,
        timestamp: 1_700_000_000,
        n_bits: 0x2100_ffff,
        nonce: 0,
    };

    let result = validate_block(&header, &transactions);
    assert_eq!(result, Err(ValidationError::InvalidProofOfWork));
}

#[test]
fn test_block_weight_exceeded_error() {
    let mut large_script = Vec::new();
    large_script.resize(1_100_000, 0x41);

    let large_tx = Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig: large_script.clone(),
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value: 50 * 100_000_000,
            script_pubkey: large_script,
        }],
        witnesses: vec![Witness {
            stack_items: Vec::new(),
        }],
        locktime: 0,
    };

    let transactions = vec![large_tx];
    let header = header_with_valid_pow(&transactions);

    let result = validate_block(&header, &transactions);
    assert_eq!(result, Err(ValidationError::BlockWeightExceeded));
}

#[test]
fn test_duplicate_transactions_error() {
    let coinbase = sample_coinbase(50 * 100_000_000);
    let tx = sample_transaction(1, 10_000);
    let duplicate = tx.clone();

    let transactions = vec![coinbase, tx, duplicate];
    let header = header_with_valid_pow(&transactions);

    let result = validate_block(&header, &transactions);
    assert_eq!(result, Err(ValidationError::DuplicateTransaction));
}

#[test]
fn test_transaction_structure_invalid_error() {
    let coinbase = sample_coinbase(50 * 100_000_000);

    let invalid_tx = Transaction {
        version: 1,
        inputs: Vec::new(),
        outputs: vec![TxOutput {
            value: 10_000,
            script_pubkey: vec![0x51],
        }],
        witnesses: Vec::new(),
        locktime: 0,
    };

    let transactions = vec![coinbase, invalid_tx];
    let header = header_with_valid_pow(&transactions);

    let result = validate_block(&header, &transactions);
    assert_eq!(result, Err(ValidationError::TransactionStructureInvalid));
}

#[test]
fn test_subsidy_calculation_halvings() {
    let height0 = 0;
    let height1 = 840_000;
    let height2 = 1_680_000;

    let s0 = calculate_subsidy(height0);
    let s1 = calculate_subsidy(height1);
    let s2 = calculate_subsidy(height2);

    assert_eq!(s0, 50 * 100_000_000);
    assert_eq!(s1, 25 * 100_000_000);
    assert_eq!(s2, 1_250_000_000);
}

#[test]
fn test_coinbase_reward_validation_ok() {
    let coinbase = sample_coinbase(50 * 100_000_000);

    let result = validate_coinbase_reward(&coinbase, 0, 0);
    assert_eq!(result, Ok(()));
}

#[test]
fn test_coinbase_reward_too_high_error() {
    let coinbase = sample_coinbase(51 * 100_000_000);

    let result = validate_coinbase_reward(&coinbase, 0, 0);
    assert_eq!(result, Err(ValidationError::CoinbaseRewardTooHigh));
}

// --- Timestamp Validation Tests ---

#[test]
fn test_timestamp_validation_ok() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut prev_headers = Vec::new();
    for i in 0..11 {
        let mut h = header_with_valid_pow(&[]);
        h.timestamp = now - 1000 + i;
        prev_headers.push(h);
    }

    let mut header = header_with_valid_pow(&[]);
    header.timestamp = now; // Current time, should be > MTP

    assert_eq!(validate_timestamp(&header, &prev_headers), Ok(()));
}

#[test]
fn test_timestamp_too_old() {
    let now = 1_700_000_000; // Fixed time

    let mut prev_headers = Vec::new();
    // Create 11 headers with timestamps around 'now'
    for i in 0..11 {
        let mut h = header_with_valid_pow(&[]);
        h.timestamp = now - 100 + i; // timestamps: now-100 to now-90
        prev_headers.push(h);
    }
    // Median of 11 items is item at index 5 (sorted).
    // Timestamps are sorted by construction here.
    // Median is now-100+5 = now-95.

    let mut header = header_with_valid_pow(&[]);
    header.timestamp = now - 95; // Equal to median
    assert_eq!(
        validate_timestamp(&header, &prev_headers),
        Err(ValidationError::TimestampTooOld)
    );

    header.timestamp = now - 96; // Less than median
    assert_eq!(
        validate_timestamp(&header, &prev_headers),
        Err(ValidationError::TimestampTooOld)
    );
}

#[test]
fn test_timestamp_too_far_future() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut header = header_with_valid_pow(&[]);
    header.timestamp = now + 7201; // 2 hours + 1 second

    // We pass empty prev_headers because we only test future check here?
    // But empty prev_headers returns Ok(()) early for genesis.
    // So we need at least one prev header.
    let prev = header_with_valid_pow(&[]);

    assert_eq!(
        validate_timestamp(&header, &[prev]),
        Err(ValidationError::TimestampTooFarFuture)
    );
}

#[test]
fn test_genesis_timestamp() {
    let header = header_with_valid_pow(&[]);
    assert_eq!(validate_timestamp(&header, &[]), Ok(()));
}

// --- Witness Commitment Tests ---

#[test]
fn test_witness_commitment_valid() {
    // 1. Create a transaction with witness data
    let mut tx = sample_transaction(1, 1000);
    tx.witnesses[0].stack_items.push(vec![1, 2, 3]);
    let wtxid = tx.wtxid();

    // 2. Create coinbase with correct commitment
    let mut coinbase = sample_coinbase(50 * 100_000_000);

    // Witness root of [wtxid] (excluding coinbase)
    // Actually validation skips coinbase (index 0) in wtxids list.
    // So if we have [coinbase, tx], we compute root of [tx.wtxid()].
    let witness_root = compute_witness_merkle_root(&[wtxid]);
    let reserved = [0u8; 32]; // Default reserved value

    let mut commitment_data = Vec::new();
    commitment_data.extend_from_slice(&witness_root);
    commitment_data.extend_from_slice(&reserved);
    let commitment_hash = sha256d(&commitment_data);

    let mut script_pubkey = vec![0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];
    script_pubkey.extend_from_slice(&commitment_hash);

    coinbase.outputs.push(TxOutput {
        value: 0,
        script_pubkey,
    });

    let transactions = vec![coinbase.clone(), tx];

    assert_eq!(
        validate_witness_commitment(&coinbase, &transactions),
        Ok(())
    );
}

#[test]
fn test_witness_commitment_invalid_hash() {
    let mut tx = sample_transaction(1, 1000);
    tx.witnesses[0].stack_items.push(vec![1, 2, 3]);

    let mut coinbase = sample_coinbase(50 * 100_000_000);

    // Invalid hash
    let mut script_pubkey = vec![0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];
    script_pubkey.extend_from_slice(&[0x00; 32]);

    coinbase.outputs.push(TxOutput {
        value: 0,
        script_pubkey,
    });

    let transactions = vec![coinbase.clone(), tx];

    assert_eq!(
        validate_witness_commitment(&coinbase, &transactions),
        Err(ValidationError::InvalidWitnessCommitment)
    );
}

#[test]
fn test_missing_witness_commitment() {
    let mut tx = sample_transaction(1, 1000);
    tx.witnesses[0].stack_items.push(vec![1, 2, 3]);

    let coinbase = sample_coinbase(50 * 100_000_000);
    // No commitment output

    let transactions = vec![coinbase.clone(), tx];

    assert_eq!(
        validate_witness_commitment(&coinbase, &transactions),
        Err(ValidationError::MissingWitnessCommitment)
    );
}

#[test]
fn test_no_witness_commitment_needed() {
    let tx = sample_transaction(1, 1000);
    // No witness data in tx

    let coinbase = sample_coinbase(50 * 100_000_000);

    let transactions = vec![coinbase.clone(), tx];

    assert_eq!(
        validate_witness_commitment(&coinbase, &transactions),
        Ok(())
    );
}

// --- Coinbase Height Tests ---

#[test]
fn test_coinbase_height_valid() {
    let mut coinbase = sample_coinbase(50 * 100_000_000);
    // Encode height 100
    // 100 = 0x64. Length 1.
    coinbase.inputs[0].script_sig = vec![0x01, 0x64];

    assert_eq!(validate_coinbase_height(&coinbase, 100), Ok(()));
}

#[test]
fn test_coinbase_height_mismatch() {
    let mut coinbase = sample_coinbase(50 * 100_000_000);
    // Encode height 100
    coinbase.inputs[0].script_sig = vec![0x01, 0x64];

    assert_eq!(
        validate_coinbase_height(&coinbase, 101),
        Err(ValidationError::WrongHeightCommitment)
    );
}

#[test]
fn test_missing_height_commitment() {
    let mut coinbase = sample_coinbase(50 * 100_000_000);
    coinbase.inputs[0].script_sig = Vec::new();

    assert_eq!(
        validate_coinbase_height(&coinbase, 100),
        Err(ValidationError::MissingHeightCommitment)
    );
}
