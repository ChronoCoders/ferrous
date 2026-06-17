use crate::consensus::transaction::{BlindingFactor, PedersenCommitment, RangeProof};
use bulletproofs::{BulletproofGens, PedersenGens, RangeProof as BpRangeProof};
use curve25519_dalek_ng::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek_ng::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek_ng::scalar::Scalar;
use curve25519_dalek_ng::traits::Identity;
use merlin::Transcript;
use rand::RngCore;
use sha2::{Digest, Sha512};
use std::sync::OnceLock;

const RANGE_BITS: usize = 64;
const TRANSCRIPT_LABEL: &[u8] = b"Ferrous/RangeProof";
const AMOUNT_PLAINTEXT_LEN: usize = 40;
const AMOUNT_KDF_LABEL: &[u8] = b"Ferrous/amount-kdf";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitmentError {
    ProofGeneration,
    ProofVerification,
    InvalidProofEncoding,
}

fn h_generator() -> RistrettoPoint {
    static H: OnceLock<RistrettoPoint> = OnceLock::new();
    *H.get_or_init(|| {
        let mut hasher = Sha512::new();
        hasher.update(b"Ferrous/H");
        let digest = hasher.finalize();
        let mut wide = [0u8; 64];
        wide.copy_from_slice(&digest);
        RistrettoPoint::from_uniform_bytes(&wide)
    })
}

fn pedersen_gens() -> PedersenGens {
    PedersenGens {
        B: RISTRETTO_BASEPOINT_POINT,
        B_blinding: h_generator(),
    }
}

fn bp_gens() -> &'static BulletproofGens {
    static BP_GENS: OnceLock<BulletproofGens> = OnceLock::new();
    BP_GENS.get_or_init(|| BulletproofGens::new(RANGE_BITS, 1))
}

fn scalar_of(blinding: &BlindingFactor) -> Scalar {
    Scalar::from_bytes_mod_order(blinding.0)
}

pub fn commit(value: u64, blinding: &BlindingFactor) -> PedersenCommitment {
    let point = pedersen_gens().commit(Scalar::from(value), scalar_of(blinding));
    PedersenCommitment(point.compress())
}

pub fn verify_balance(
    inputs: &[PedersenCommitment],
    outputs: &[PedersenCommitment],
    fee: u64,
) -> bool {
    let mut sum_in = RistrettoPoint::identity();
    for c in inputs {
        match c.0.decompress() {
            Some(p) => sum_in += p,
            None => return false,
        }
    }

    let mut sum_out = RistrettoPoint::identity();
    for c in outputs {
        match c.0.decompress() {
            Some(p) => sum_out += p,
            None => return false,
        }
    }

    let fee_point = Scalar::from(fee) * RISTRETTO_BASEPOINT_POINT;
    sum_in == sum_out + fee_point
}

pub fn generate_range_proof(
    value: u64,
    blinding: &BlindingFactor,
) -> Result<RangeProof, CommitmentError> {
    let pc_gens = pedersen_gens();
    let mut transcript = Transcript::new(TRANSCRIPT_LABEL);
    let (proof, _commitment) = BpRangeProof::prove_single(
        bp_gens(),
        &pc_gens,
        &mut transcript,
        value,
        &scalar_of(blinding),
        RANGE_BITS,
    )
    .map_err(|_| CommitmentError::ProofGeneration)?;
    Ok(RangeProof(proof.to_bytes()))
}

pub fn verify_range_proof(
    commitment: &PedersenCommitment,
    proof: &RangeProof,
) -> Result<(), CommitmentError> {
    let pc_gens = pedersen_gens();
    let bp_proof =
        BpRangeProof::from_bytes(&proof.0).map_err(|_| CommitmentError::InvalidProofEncoding)?;
    let mut transcript = Transcript::new(TRANSCRIPT_LABEL);
    bp_proof
        .verify_single(
            bp_gens(),
            &pc_gens,
            &mut transcript,
            &commitment.0,
            RANGE_BITS,
        )
        .map_err(|_| CommitmentError::ProofVerification)
}

fn amount_keystream(shared: &CompressedRistretto, len: usize) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(AMOUNT_KDF_LABEL);
    hasher.update(shared.as_bytes());
    let mut reader = hasher.finalize_xof();
    let mut out = vec![0u8; len];
    reader.fill(&mut out);
    out
}

pub fn encrypt_amount(
    value: u64,
    blinding: &BlindingFactor,
    recipient_view_pubkey: &RistrettoPoint,
) -> (Vec<u8>, [u8; 32]) {
    let mut wide = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut wide);
    let ephemeral_scalar = Scalar::from_bytes_mod_order_wide(&wide);
    let ephemeral_pubkey = ephemeral_scalar * RISTRETTO_BASEPOINT_POINT;
    let shared = (ephemeral_scalar * recipient_view_pubkey).compress();

    let keystream = amount_keystream(&shared, AMOUNT_PLAINTEXT_LEN);

    let mut plaintext = [0u8; AMOUNT_PLAINTEXT_LEN];
    plaintext[..8].copy_from_slice(&value.to_le_bytes());
    plaintext[8..].copy_from_slice(&blinding.0);

    let encrypted: Vec<u8> = plaintext
        .iter()
        .zip(keystream.iter())
        .map(|(p, k)| p ^ k)
        .collect();

    (encrypted, ephemeral_pubkey.compress().to_bytes())
}

pub fn decrypt_amount(
    encrypted_amount: &[u8],
    ephemeral_pubkey: &[u8; 32],
    view_scalar: &Scalar,
) -> Option<(u64, BlindingFactor)> {
    if encrypted_amount.len() != AMOUNT_PLAINTEXT_LEN {
        return None;
    }
    let ephemeral_point = CompressedRistretto(*ephemeral_pubkey).decompress()?;
    let shared = (view_scalar * ephemeral_point).compress();
    let keystream = amount_keystream(&shared, AMOUNT_PLAINTEXT_LEN);

    let plaintext: Vec<u8> = encrypted_amount
        .iter()
        .zip(keystream.iter())
        .map(|(c, k)| c ^ k)
        .collect();

    let mut value_bytes = [0u8; 8];
    value_bytes.copy_from_slice(&plaintext[..8]);
    let value = u64::from_le_bytes(value_bytes);

    let mut blind_bytes = [0u8; 32];
    blind_bytes.copy_from_slice(&plaintext[8..]);

    Some((value, BlindingFactor(blind_bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::keys::derive_view_key;

    #[test]
    fn test_ecdh_encrypt_decrypt_roundtrip() {
        let (view_scalar, view_pubkey) = derive_view_key(&[0x42u8; 64]);
        let value = 7_777_777u64;
        let blind = BlindingFactor([13u8; 32]);

        let (enc, eph) = encrypt_amount(value, &blind, &view_pubkey);
        assert_eq!(enc.len(), AMOUNT_PLAINTEXT_LEN);

        let (dv, db) = decrypt_amount(&enc, &eph, &view_scalar).expect("decrypt must succeed");
        assert_eq!(dv, value);
        assert_eq!(db, blind);

        let (wrong_scalar, _) = derive_view_key(&[0x99u8; 64]);
        assert_ne!(
            decrypt_amount(&enc, &eph, &wrong_scalar),
            Some((value, blind))
        );

        assert!(decrypt_amount(&enc[..39], &eph, &view_scalar).is_none());
    }
}
