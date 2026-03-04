use crate::consensus::transaction::{Transaction, TxOutput};
use crate::primitives::hash::{sha256d, tagged_hash, Hash256};
use crate::primitives::serialize::Encode;

pub const SIGHASH_ALL: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SighashError {
    InputIndexOutOfBounds,
    SpentOutputsMismatch,
}

/// Compute hash of all previous outputs
fn hash_prevouts(tx: &Transaction) -> Hash256 {
    let mut data = Vec::new();
    for input in &tx.inputs {
        data.extend_from_slice(&input.prev_txid);
        data.extend_from_slice(&input.prev_index.encode());
    }
    sha256d(&data)
}

/// Compute hash of all input amounts
fn hash_amounts(spent_outputs: &[TxOutput]) -> Hash256 {
    let mut data = Vec::new();
    for output in spent_outputs {
        data.extend_from_slice(&output.value.encode());
    }
    sha256d(&data)
}

/// Compute hash of all scriptPubKeys being spent
fn hash_scriptpubkeys(spent_outputs: &[TxOutput]) -> Hash256 {
    let mut data = Vec::new();
    for output in spent_outputs {
        data.extend_from_slice(&output.script_pubkey.encode());
    }
    sha256d(&data)
}

/// Compute hash of all input sequences
fn hash_sequences(tx: &Transaction) -> Hash256 {
    let mut data = Vec::new();
    for input in &tx.inputs {
        data.extend_from_slice(&input.sequence.encode());
    }
    sha256d(&data)
}

/// Compute hash of all outputs
fn hash_outputs(tx: &Transaction) -> Hash256 {
    let mut data = Vec::new();
    for output in &tx.outputs {
        data.extend_from_slice(&output.encode());
    }
    sha256d(&data)
}

/// Compute signature hash for input
pub fn compute_sighash(
    tx: &Transaction,
    input_index: usize,
    spent_outputs: &[TxOutput],
) -> Result<Hash256, SighashError> {
    // Validate inputs
    if input_index >= tx.inputs.len() {
        return Err(SighashError::InputIndexOutOfBounds);
    }

    if spent_outputs.len() != tx.inputs.len() {
        return Err(SighashError::SpentOutputsMismatch);
    }

    // Build sighash message
    let mut message = Vec::new();

    // version (u32 LE)
    message.extend_from_slice(&tx.version.encode());

    // locktime (u32 LE)
    message.extend_from_slice(&tx.locktime.encode());

    // hash_prevouts (32 bytes)
    message.extend_from_slice(&hash_prevouts(tx));

    // hash_amounts (32 bytes)
    message.extend_from_slice(&hash_amounts(spent_outputs));

    // hash_scriptpubkeys (32 bytes)
    message.extend_from_slice(&hash_scriptpubkeys(spent_outputs));

    // hash_sequences (32 bytes)
    message.extend_from_slice(&hash_sequences(tx));

    // hash_outputs (32 bytes)
    message.extend_from_slice(&hash_outputs(tx));

    // input_index (u32 LE)
    message.extend_from_slice(&(input_index as u32).encode());

    // sighash_type (u8)
    message.push(SIGHASH_ALL);

    // Compute tagged hash
    Ok(tagged_hash("FerrousSighash", &message))
}
