use crate::consensus::transaction::{BlindingFactor, PedersenCommitment, RangeProof};
use bulletproofs::{BulletproofGens, PedersenGens, RangeProof as BpRangeProof};
use curve25519_dalek_ng::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek_ng::ristretto::RistrettoPoint;
use curve25519_dalek_ng::scalar::Scalar;
use curve25519_dalek_ng::traits::Identity;
use merlin::Transcript;
use sha2::{Digest, Sha512};
use std::sync::OnceLock;

const RANGE_BITS: usize = 64;
const TRANSCRIPT_LABEL: &[u8] = b"Ferrous/RangeProof";

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
    let bp_gens = BulletproofGens::new(RANGE_BITS, 1);
    let mut transcript = Transcript::new(TRANSCRIPT_LABEL);
    let (proof, _commitment) = BpRangeProof::prove_single(
        &bp_gens,
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
    let bp_gens = BulletproofGens::new(RANGE_BITS, 1);
    let bp_proof =
        BpRangeProof::from_bytes(&proof.0).map_err(|_| CommitmentError::InvalidProofEncoding)?;
    let mut transcript = Transcript::new(TRANSCRIPT_LABEL);
    bp_proof
        .verify_single(
            &bp_gens,
            &pc_gens,
            &mut transcript,
            &commitment.0,
            RANGE_BITS,
        )
        .map_err(|_| CommitmentError::ProofVerification)
}
