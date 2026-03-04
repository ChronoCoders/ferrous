//! Merkle tree construction for block validation

use crate::primitives::hash::{sha256d, Hash256};

/// Compute merkle root from transaction IDs
pub fn compute_merkle_root(txids: &[Hash256]) -> Hash256 {
    if txids.is_empty() {
        return [0u8; 32];
    }

    let mut level: Vec<Hash256> = txids.to_vec();

    while level.len() > 1 {
        let mut next_level = Vec::new();

        for i in (0..level.len()).step_by(2) {
            let left = level[i];
            let right = if i + 1 < level.len() {
                level[i + 1]
            } else {
                level[i] // Duplicate last if odd
            };

            let mut data = Vec::with_capacity(64);
            data.extend_from_slice(&left);
            data.extend_from_slice(&right);

            next_level.push(sha256d(&data));
        }

        level = next_level;
    }

    level[0]
}

/// Compute witness merkle root (for coinbase commitment)
pub fn compute_witness_merkle_root(wtxids: &[Hash256]) -> Hash256 {
    // First wtxid must be all zeros (coinbase)
    let mut txids = vec![[0u8; 32]];
    txids.extend_from_slice(wtxids);
    compute_merkle_root(&txids)
}
