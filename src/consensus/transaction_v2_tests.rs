use crate::consensus::transaction::BlindingFactor;
use crate::crypto::commitments::{
    commit, generate_range_proof, verify_balance, verify_range_proof,
};
use curve25519_dalek_ng::scalar::Scalar;

fn balancing_input_blind(x1: &BlindingFactor, x2: &BlindingFactor) -> BlindingFactor {
    let s = Scalar::from_bytes_mod_order(x1.0) + Scalar::from_bytes_mod_order(x2.0);
    BlindingFactor(s.to_bytes())
}

#[test]
fn test_commitment_roundtrip() {
    let x1 = BlindingFactor([7u8; 32]);
    let x2 = BlindingFactor([9u8; 32]);
    let x_in = balancing_input_blind(&x1, &x2);

    let input = commit(1000, &x_in);
    let out1 = commit(600, &x1);
    let out2 = commit(300, &x2);

    assert!(verify_balance(&[input], &[out1, out2], 100));
}

#[test]
fn test_range_proof_valid() {
    let blind = BlindingFactor([3u8; 32]);
    let proof = generate_range_proof(1000, &blind).expect("generate small");
    let commitment = commit(1000, &blind);
    assert!(verify_range_proof(&commitment, &proof).is_ok());

    let blind_big = BlindingFactor([4u8; 32]);
    let big = u64::MAX - 1;
    let proof_big = generate_range_proof(big, &blind_big).expect("generate big");
    let commitment_big = commit(big, &blind_big);
    assert!(verify_range_proof(&commitment_big, &proof_big).is_ok());
}

#[test]
fn test_range_proof_invalid() {
    let blind = BlindingFactor([3u8; 32]);
    let proof = generate_range_proof(1000, &blind).expect("generate");

    let tampered = commit(2000, &blind);
    assert!(verify_range_proof(&tampered, &proof).is_err());
}

#[test]
fn test_balance_invalid() {
    let x1 = BlindingFactor([7u8; 32]);
    let x2 = BlindingFactor([9u8; 32]);
    let x_in = balancing_input_blind(&x1, &x2);

    let input = commit(1000, &x_in);
    let out1 = commit(600, &x1);
    let out2 = commit(300, &x2);

    assert!(!verify_balance(&[input], &[out1, out2], 200));
}
