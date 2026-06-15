use crate::consensus::block::Block;
use crate::consensus::transaction::{BlindingFactor, TxKind};
use crate::consensus::utxo::OutPoint;
use crate::crypto::commitments::{commit, decrypt_amount};
use curve25519_dalek_ng::scalar::Scalar;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct V2UtxoCache {
    entries: HashMap<OutPoint, (u64, BlindingFactor)>,
}

impl V2UtxoCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn get(&self, outpoint: &OutPoint) -> Option<&(u64, BlindingFactor)> {
        self.entries.get(outpoint)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn scan_v2_outputs(block: &Block, view_scalar: &Scalar, cache: &mut V2UtxoCache) {
    for tx in &block.transactions {
        let v2 = match tx {
            TxKind::V2(v2) => v2,
            TxKind::V1(_) => continue,
        };

        let txid = v2.txid();
        for (vout, output) in v2.outputs.iter().enumerate() {
            let Some((value, blinding)) = decrypt_amount(
                &output.encrypted_amount,
                &output.ephemeral_pubkey,
                view_scalar,
            ) else {
                continue;
            };

            if commit(value, &blinding) != output.commitment {
                continue;
            }

            cache.entries.insert(
                OutPoint {
                    txid,
                    vout: vout as u32,
                },
                (value, blinding),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::block::{Block, BlockHeader};
    use crate::consensus::transaction::{
        RangeProof, TransactionV2, TxInputV2, TxOutputV2, TX_VERSION_V2,
    };
    use crate::crypto::commitments::encrypt_amount;
    use crate::wallet::keys::derive_view_key;

    #[test]
    fn test_v2_scan_finds_output() {
        let (view_scalar, view_pubkey) = derive_view_key(&[0x55u8; 64]);
        let value = 12_345u64;
        let blind = BlindingFactor([7u8; 32]);
        let commitment = commit(value, &blind);
        let (enc, eph) = encrypt_amount(value, &blind, &view_pubkey);

        let tx = TransactionV2 {
            version: TX_VERSION_V2,
            inputs: vec![TxInputV2 {
                prev_txid: [1u8; 32],
                prev_index: 0,
                script_sig: vec![],
                sequence: 0xFFFF_FFFF,
            }],
            outputs: vec![TxOutputV2 {
                commitment,
                range_proof: RangeProof(vec![]),
                script_pubkey: vec![],
                encrypted_amount: enc,
                ephemeral_pubkey: eph,
            }],
            fee: 0,
            locktime: 0,
        };
        let txid = tx.txid();

        let block = Block {
            header: BlockHeader {
                version: 1,
                prev_block_hash: [0u8; 32],
                merkle_root: [0u8; 32],
                timestamp: 0,
                n_bits: 0,
                nonce: 0,
            },
            transactions: vec![TxKind::V2(tx)],
        };

        let mut cache = V2UtxoCache::new();
        assert!(cache.is_empty());

        scan_v2_outputs(&block, &view_scalar, &mut cache);
        assert_eq!(cache.len(), 1);

        let found = cache
            .get(&OutPoint { txid, vout: 0 })
            .expect("output found");
        assert_eq!(found.0, value);
        assert_eq!(found.1, blind);

        let (wrong_scalar, _) = derive_view_key(&[0xEEu8; 64]);
        let mut empty = V2UtxoCache::new();
        scan_v2_outputs(&block, &wrong_scalar, &mut empty);
        assert!(empty.is_empty());
    }
}
