use crate::consensus::transaction::Transaction;
use crate::primitives::hash::{sha256d, Hash256};
use crate::primitives::serialize::{Decode, DecodeError, Encode};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// 256-bit unsigned integer used for difficulty targets and hash comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct U256(pub [u8; 32]);

impl U256 {
    /// Constructs a 256-bit integer from little-endian bytes.
    pub fn from_le_bytes(bytes: [u8; 32]) -> Self {
        U256(bytes)
    }
}

impl Ord for U256 {
    fn cmp(&self, other: &Self) -> Ordering {
        for (a, b) in self.0.iter().rev().zip(other.0.iter().rev()) {
            match a.cmp(b) {
                Ordering::Equal => continue,
                non_eq => return non_eq,
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for U256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Errors that can occur while decoding difficulty targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetError {
    /// Mantissa encodes a negative target (most significant bit set).
    NegativeEncoding,
    /// Exponent exceeds the maximum supported value for a 256-bit target.
    ExponentTooLarge,
    /// The decoded target value does not fit within 256 bits.
    Overflow,
}

/// Block header used in the Ferrous Network consensus layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u32,
    pub prev_block_hash: Hash256,
    pub merkle_root: Hash256,
    pub timestamp: u64,
    pub n_bits: u32,
    pub nonce: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockData {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    pub height: u32,
    pub cumulative_work: u128,
}

const HEADER_SIZE: usize = 88;

impl Encode for BlockHeader {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_SIZE);

        out.extend_from_slice(&self.version.encode());
        out.extend_from_slice(&self.prev_block_hash.encode());
        out.extend_from_slice(&self.merkle_root.encode());
        out.extend_from_slice(&self.timestamp.encode());
        out.extend_from_slice(&self.n_bits.encode());
        out.extend_from_slice(&self.nonce.encode());

        out
    }

    fn encoded_size(&self) -> usize {
        HEADER_SIZE
    }
}

impl Decode for BlockHeader {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (version, c1) = u32::decode(bytes)?;
        let (prev_block_hash, c2) = <[u8; 32]>::decode(&bytes[c1..])?;
        let (merkle_root, c3) = <[u8; 32]>::decode(&bytes[c1 + c2..])?;
        let (timestamp, c4) = u64::decode(&bytes[c1 + c2 + c3..])?;
        let (n_bits, c5) = u32::decode(&bytes[c1 + c2 + c3 + c4..])?;
        let (nonce, c6) = u64::decode(&bytes[c1 + c2 + c3 + c4 + c5..])?;

        let consumed = c1 + c2 + c3 + c4 + c5 + c6;
        debug_assert_eq!(consumed, HEADER_SIZE);

        Ok((
            BlockHeader {
                version,
                prev_block_hash,
                merkle_root,
                timestamp,
                n_bits,
                nonce,
            },
            consumed,
        ))
    }
}

impl BlockHeader {
    /// Computes the double SHA-256 hash of the encoded block header.
    pub fn hash(&self) -> Hash256 {
        let encoded = self.encode();
        sha256d(&encoded)
    }

    /// Decodes the compact difficulty representation (nBits) into a 256-bit target.
    ///
    /// The format is an exponent byte followed by a 24-bit mantissa. The target
    /// is computed as:
    ///
    /// `target = mantissa × 256^(exponent - 3)`
    ///
    /// with additional constraints:
    /// - `mantissa < 0x800000` (non-negative encoding)
    /// - `exponent <= 32` (fits within 256 bits)
    pub fn target(&self) -> Result<U256, TargetError> {
        let n_bits = self.n_bits;
        let exponent = (n_bits >> 24) as u8;
        let mantissa = n_bits & 0x00FF_FFFF;

        if mantissa >= 0x0080_0000 {
            return Err(TargetError::NegativeEncoding);
        }

        if exponent > 32 {
            return Err(TargetError::ExponentTooLarge);
        }

        let mut bytes = [0u8; 32];

        if exponent > 3 {
            let shift_bytes = usize::from(exponent - 3);

            if shift_bytes + 3 > 32 {
                return Err(TargetError::Overflow);
            }

            let mant_bytes = [
                (mantissa & 0xFF) as u8,
                ((mantissa >> 8) & 0xFF) as u8,
                ((mantissa >> 16) & 0xFF) as u8,
            ];

            for (i, b) in mant_bytes.iter().enumerate() {
                bytes[shift_bytes + i] = *b;
            }
        } else {
            let shift = 8 * (3_u32.saturating_sub(u32::from(exponent)));
            let value = mantissa >> shift;

            let mant_bytes = [
                (value & 0xFF) as u8,
                ((value >> 8) & 0xFF) as u8,
                ((value >> 16) & 0xFF) as u8,
            ];

            for (i, b) in mant_bytes.iter().enumerate() {
                if i < 32 {
                    bytes[i] = *b;
                }
            }
        }

        Ok(U256::from_le_bytes(bytes))
    }

    /// Verifies that the block header hash satisfies the proof-of-work target.
    ///
    /// Returns `Ok(true)` when `hash <= target`, `Ok(false)` otherwise.
    pub fn check_proof_of_work(&self) -> Result<bool, TargetError> {
        let target = self.target()?;
        let hash = self.hash();
        let hash_value = U256::from_le_bytes(hash);
        Ok(hash_value <= target)
    }
}
