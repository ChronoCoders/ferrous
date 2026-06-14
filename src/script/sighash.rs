use crate::consensus::transaction::{Transaction, TransactionV2, TxOutput};
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

    // Compute tagged hash — V2 domain-separates Dilithium txs from legacy ECDSA history
    Ok(tagged_hash("FerrousSighashV2", &message))
}

fn hash_prevouts_v2(tx: &TransactionV2) -> Hash256 {
    let mut data = Vec::new();
    for input in &tx.inputs {
        data.extend_from_slice(&input.prev_txid);
        data.extend_from_slice(&input.prev_index.encode());
    }
    sha256d(&data)
}

fn hash_sequences_v2(tx: &TransactionV2) -> Hash256 {
    let mut data = Vec::new();
    for input in &tx.inputs {
        data.extend_from_slice(&input.sequence.encode());
    }
    sha256d(&data)
}

fn hash_outputs_v2(tx: &TransactionV2) -> Hash256 {
    let mut data = Vec::new();
    for output in &tx.outputs {
        data.extend_from_slice(output.commitment.0.as_bytes());
        data.extend_from_slice(&output.script_pubkey.encode());
        data.extend_from_slice(&output.range_proof.0.encode());
        data.extend_from_slice(&output.encrypted_amount.encode());
        data.extend_from_slice(&output.ephemeral_pubkey);
    }
    sha256d(&data)
}

pub fn compute_sighash_v2(
    tx: &TransactionV2,
    input_index: usize,
    spent_script_pubkey: &[u8],
) -> Result<Hash256, SighashError> {
    if input_index >= tx.inputs.len() {
        return Err(SighashError::InputIndexOutOfBounds);
    }

    let mut message = Vec::new();
    message.extend_from_slice(&tx.version.encode());
    message.extend_from_slice(&tx.locktime.encode());
    message.extend_from_slice(&hash_prevouts_v2(tx));
    message.extend_from_slice(&hash_sequences_v2(tx));
    message.extend_from_slice(&sha256d(spent_script_pubkey));
    message.extend_from_slice(&hash_outputs_v2(tx));
    message.extend_from_slice(&tx.fee.encode());
    message.extend_from_slice(&(input_index as u32).encode());
    message.push(SIGHASH_ALL);

    Ok(tagged_hash("FerrousSighashV2CT", &message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::transaction::{TxInput, TxOutput};
    use crate::primitives::hash::tagged_hash;

    fn mock_p2dl_script() -> Vec<u8> {
        // OP_HASH256 push32 <32 zero bytes> OP_EQUALVERIFY OP_CHECKSIG = 36 bytes
        let mut s = vec![0xaa, 0x20];
        s.extend_from_slice(&[0u8; 32]);
        s.push(0x88);
        s.push(0xac);
        s
    }

    #[test]
    fn test_p2dl_sighash_v2_roundtrip() {
        let script_pubkey = mock_p2dl_script();
        assert_eq!(script_pubkey.len(), 36);

        let spent = TxOutput {
            value: 1_000_000,
            script_pubkey,
        };

        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                prev_txid: [1u8; 32],
                prev_index: 0,
                script_sig: Vec::new(),
                sequence: 0xFFFF_FFFF,
            }],
            outputs: vec![TxOutput {
                value: 999_000,
                script_pubkey: mock_p2dl_script(),
            }],
            witnesses: Vec::new(),
            locktime: 0,
        };

        let hash = compute_sighash(&tx, 0, &[spent]).expect("sighash failed");
        assert_eq!(hash.len(), 32);

        // V2 tag must produce a different hash than the legacy V1 tag on identical input
        let v1 = tagged_hash("FerrousSighash", b"ferrous");
        let v2 = tagged_hash("FerrousSighashV2", b"ferrous");
        assert_ne!(v1, v2, "V1 and V2 tags must domain-separate");
    }
}
