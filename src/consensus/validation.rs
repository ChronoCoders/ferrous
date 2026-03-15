use std::collections::HashSet;

use crate::consensus::block::BlockHeader;
use crate::consensus::merkle::{compute_merkle_root, compute_witness_merkle_root};
use crate::consensus::transaction::{Transaction, MAX_MONEY};
use crate::primitives::hash::sha256d;

pub const MAX_BLOCK_WEIGHT: u64 = 4_000_000;
pub const COINBASE_MATURITY: u64 = 100;
/// Maximum number of seconds a block timestamp may exceed the node's wall-clock time.
/// Exposed as a constant so tests and per-network config can reference it directly.
pub const MAX_FUTURE_BLOCK_TIME: u64 = 7_200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    NoTransactions,
    NoCoinbase,
    MultipleCoinbases,
    InvalidMerkleRoot,
    InvalidProofOfWork,
    BlockWeightExceeded,
    CoinbaseRewardTooHigh,
    InvalidCoinbaseStructure,
    DuplicateTransaction,
    TransactionStructureInvalid,
    TimestampTooOld,
    TimestampTooFarFuture,
    InvalidWitnessCommitment,
    MissingWitnessCommitment,
    MissingHeightCommitment,
    InvalidHeightCommitment,
    WrongHeightCommitment,
}

/// Validate block structure and consensus rules
pub fn validate_block(
    header: &BlockHeader,
    transactions: &[Transaction],
) -> Result<(), ValidationError> {
    // 1. Must have at least one transaction
    if transactions.is_empty() {
        return Err(ValidationError::NoTransactions);
    }

    // 2. First tx must be coinbase, rest must not be
    if !is_coinbase(&transactions[0]) {
        return Err(ValidationError::NoCoinbase);
    }

    for tx in &transactions[1..] {
        if is_coinbase(tx) {
            return Err(ValidationError::MultipleCoinbases);
        }
    }

    // 3. Verify merkle root
    let txids: Vec<_> = transactions.iter().map(|tx| tx.txid()).collect();

    let computed_root = compute_merkle_root(&txids);
    if computed_root != header.merkle_root {
        return Err(ValidationError::InvalidMerkleRoot);
    }

    // 4. Verify proof of work
    if !header
        .check_proof_of_work()
        .map_err(|_| ValidationError::InvalidProofOfWork)?
    {
        return Err(ValidationError::InvalidProofOfWork);
    }

    // 5. Check block weight
    let weight = calculate_block_weight(transactions);
    if weight > MAX_BLOCK_WEIGHT {
        return Err(ValidationError::BlockWeightExceeded);
    }

    // 6. Validate all transaction structures
    for tx in transactions {
        tx.check_structure()
            .map_err(|_| ValidationError::TransactionStructureInvalid)?;
    }

    // Verify witness commitment
    validate_witness_commitment(&transactions[0], transactions)?;

    // 7. Check for duplicate transactions
    let mut seen_txids = HashSet::new();
    for tx in transactions {
        if !seen_txids.insert(tx.txid()) {
            return Err(ValidationError::DuplicateTransaction);
        }
    }

    Ok(())
}

fn is_coinbase(tx: &Transaction) -> bool {
    tx.inputs.len() == 1
        && tx.inputs[0].prev_txid == [0u8; 32]
        && tx.inputs[0].prev_index == 0xFFFF_FFFF
}

fn calculate_block_weight(transactions: &[Transaction]) -> u64 {
    let mut weight = 0u64;

    for tx in transactions {
        let base_size = tx.encode_without_witness().len() as u64;
        let total_size = tx.encode_with_witness().len() as u64;

        // Weight = base_size * 3 + total_size
        weight += base_size * 3 + total_size;
    }

    weight
}

/// Validate coinbase reward
pub fn validate_coinbase_reward(
    coinbase: &Transaction,
    block_fees: u64,
    block_height: u32,
) -> Result<(), ValidationError> {
    let subsidy = calculate_subsidy(block_height);
    let max_reward = subsidy
        .checked_add(block_fees)
        .ok_or(ValidationError::CoinbaseRewardTooHigh)?;

    let coinbase_value: u64 = coinbase.outputs.iter().map(|o| o.value).sum();

    if coinbase_value > MAX_MONEY {
        return Err(ValidationError::CoinbaseRewardTooHigh);
    }

    if coinbase_value > max_reward {
        return Err(ValidationError::CoinbaseRewardTooHigh);
    }

    Ok(())
}

pub fn calculate_subsidy(height: u32) -> u64 {
    const INITIAL_SUBSIDY: u64 = 50 * 100_000_000;
    const HALVING_INTERVAL: u32 = 840_000;

    let halvings = height / HALVING_INTERVAL;

    if halvings >= 64 {
        return 0;
    }

    INITIAL_SUBSIDY >> halvings
}

/// Validate block timestamp against previous blocks
pub fn validate_timestamp(
    header: &BlockHeader,
    prev_headers: &[BlockHeader], // Last 11 headers
) -> Result<(), ValidationError> {
    if prev_headers.is_empty() {
        return Ok(()); // Genesis block
    }

    // Compute MedianTimePast
    let mut timestamps: Vec<u64> = prev_headers.iter().map(|h| h.timestamp).collect();
    timestamps.sort_unstable();

    let mtp = if timestamps.len() & 1 == 0 {
        timestamps[timestamps.len() / 2 - 1]
    } else {
        timestamps[timestamps.len() / 2]
    };

    // Timestamp must be > MTP
    if header.timestamp <= mtp {
        return Err(ValidationError::TimestampTooOld);
    }

    // Timestamp must not be too far in future
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if header.timestamp > now + MAX_FUTURE_BLOCK_TIME {
        return Err(ValidationError::TimestampTooFarFuture);
    }

    Ok(())
}

/// Validate witness commitment in coinbase
pub fn validate_witness_commitment(
    coinbase: &Transaction,
    transactions: &[Transaction],
) -> Result<(), ValidationError> {
    // Check if block has witness data
    let has_witness = transactions.iter().any(|tx| tx.has_witness());

    if !has_witness {
        return Ok(()); // No witness, no commitment needed
    }

    // Find witness commitment in coinbase outputs
    let mut commitment_found = false;

    for output in &coinbase.outputs {
        if output.script_pubkey.len() >= 38
            && output.script_pubkey[0] == 0x6a  // OP_RETURN
             && output.script_pubkey[1] == 0x24  // 36 bytes
             && output.script_pubkey[2..6] == [0xaa, 0x21, 0xa9, 0xed]
        {
            commitment_found = true;

            // Extract commitment
            let commitment = &output.script_pubkey[6..38];

            // Compute witness merkle root.
            // `compute_witness_merkle_root` always prepends [0u8;32] as the coinbase
            // placeholder, so passing `wtxids[1..]` (excluding the coinbase wtxid) is
            // correct even when the slice is empty (coinbase-only block).
            let wtxids: Vec<_> = transactions.iter().map(|tx| tx.wtxid()).collect();

            let witness_root = compute_witness_merkle_root(&wtxids[1..]);

            // Witness reserved value from coinbase witness
            let reserved = if !coinbase.witnesses.is_empty()
                && !coinbase.witnesses[0].stack_items.is_empty()
            {
                &coinbase.witnesses[0].stack_items[0]
            } else {
                &[0u8; 32][..]
            };

            // Compute commitment hash
            let mut data = Vec::with_capacity(64);
            data.extend_from_slice(&witness_root);
            data.extend_from_slice(reserved);

            let computed = sha256d(&data);

            if commitment != computed {
                return Err(ValidationError::InvalidWitnessCommitment);
            }
        }
    }

    if has_witness && !commitment_found {
        return Err(ValidationError::MissingWitnessCommitment);
    }

    Ok(())
}

/// Validate coinbase contains block height
pub fn validate_coinbase_height(
    coinbase: &Transaction,
    expected_height: u32,
) -> Result<(), ValidationError> {
    if coinbase.inputs.is_empty() {
        return Err(ValidationError::InvalidCoinbaseStructure);
    }

    let script_sig = &coinbase.inputs[0].script_sig;

    if script_sig.is_empty() {
        return Err(ValidationError::MissingHeightCommitment);
    }

    // Decode height from scriptSig (script number encoding)
    let (height, _) =
        decode_script_number(script_sig).map_err(|_| ValidationError::InvalidHeightCommitment)?;

    if height != expected_height as i64 {
        return Err(ValidationError::WrongHeightCommitment);
    }

    Ok(())
}

fn decode_script_number(bytes: &[u8]) -> Result<(i64, usize), ()> {
    if bytes.is_empty() {
        return Err(());
    }

    let len = bytes[0] as usize;
    if len > 8 || len == 0 || bytes.len() < len + 1 {
        return Err(());
    }

    let data = &bytes[1..=len];

    let negative = (data[data.len() - 1] & 0x80) != 0;
    let mut value = 0i64;

    for (i, &byte) in data.iter().enumerate() {
        let b = if i == data.len() - 1 {
            byte & 0x7f
        } else {
            byte
        };
        value |= (b as i64) << (8 * i);
    }

    if negative {
        value = -value;
    }

    Ok((value, len + 1))
}
