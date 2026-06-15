use crate::consensus::block::{Block, BlockHeader};
use crate::consensus::chain::{ChainError, ChainState};
use crate::consensus::merkle::compute_merkle_root;
use crate::consensus::params::Network;
use crate::consensus::transaction::{
    BlindingFactor, RangeProof, Transaction, TransactionV2, TxInput, TxInputV2, TxKind, TxOutput,
    TxOutputV2, TX_VERSION_V2,
};
use crate::consensus::utxo::{OutPoint, UtxoEntryV2, UtxoError};
use crate::consensus::validation::validate_transaction_v2;
use crate::crypto::commitments::{
    commit, generate_range_proof, verify_balance, verify_range_proof,
};
use crate::primitives::serialize::{Decode, DecodeError, Encode};
use crate::script::sighash::compute_sighash_v2;
use crate::wallet::dilithium::DilithiumKeypair;
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

fn push_data(script: &mut Vec<u8>, data: &[u8]) {
    script.push(0x4d);
    script.extend_from_slice(&(data.len() as u16).to_le_bytes());
    script.extend_from_slice(data);
}

fn p2dl_script(pubkey: &[u8]) -> Vec<u8> {
    let hash: [u8; 32] = blake3::hash(pubkey).into();
    let mut s = vec![0xaa, 0x20];
    s.extend_from_slice(&hash);
    s.push(0x88);
    s.push(0xac);
    s
}

#[test]
fn test_txkind_v2_roundtrip() {
    let v2 = TransactionV2 {
        version: TX_VERSION_V2,
        inputs: vec![TxInputV2 {
            prev_txid: [9u8; 32],
            prev_index: 1,
            script_sig: vec![1, 2, 3],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutputV2 {
            commitment: commit(500, &BlindingFactor([1u8; 32])),
            range_proof: RangeProof(vec![4, 5, 6, 7]),
            script_pubkey: vec![0xaa, 0x20],
            encrypted_amount: vec![8, 9],
            ephemeral_pubkey: [3u8; 32],
        }],
        fee: 11,
        locktime: 0,
    };
    let k = TxKind::V2(v2);

    let enc = k.encode();
    assert_eq!(enc.len(), k.encoded_size());

    let (decoded, consumed) = TxKind::decode(&enc).unwrap();
    assert_eq!(consumed, enc.len());
    assert_eq!(decoded, k);
    assert_eq!(decoded.encode(), enc);

    let v1 = TxKind::V1(Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [1u8; 32],
            prev_index: 0,
            script_sig: vec![],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value: 50,
            script_pubkey: vec![0x51],
        }],
        witnesses: vec![],
        locktime: 0,
    });
    let e1 = v1.encode();
    let (d1, c1) = TxKind::decode(&e1).unwrap();
    assert_eq!(c1, e1.len());
    assert!(matches!(d1, TxKind::V1(_)));
    assert_eq!(d1, v1);
}

#[test]
fn test_v2_block_apply_revert() {
    let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();

    let kp = DilithiumKeypair::generate();
    let pubkey = kp.verifying_key_bytes();
    let script_pubkey = p2dl_script(&pubkey);

    let v_in = 1000u64;
    let fee = 100u64;
    let v_out = v_in - fee;
    let blind = BlindingFactor([5u8; 32]);

    let c_in = commit(v_in, &blind);
    let in_op = OutPoint {
        txid: [9u8; 32],
        vout: 0,
    };
    let in_entry = UtxoEntryV2 {
        commitment: *c_in.0.as_bytes(),
        script_pubkey: script_pubkey.clone(),
        encrypted_amount: vec![],
        ephemeral_pubkey: [0u8; 32],
        coinbase: false,
        height: 0,
    };
    chain.utxo_store_v2.put_utxo(&in_op, &in_entry).unwrap();

    let c_out = commit(v_out, &blind);
    let proof = generate_range_proof(v_out, &blind).unwrap();

    let mut tx = TransactionV2 {
        version: TX_VERSION_V2,
        inputs: vec![TxInputV2 {
            prev_txid: in_op.txid,
            prev_index: in_op.vout,
            script_sig: vec![],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutputV2 {
            commitment: c_out.clone(),
            range_proof: proof,
            script_pubkey: script_pubkey.clone(),
            encrypted_amount: vec![],
            ephemeral_pubkey: [0u8; 32],
        }],
        fee,
        locktime: 0,
    };

    let sighash = compute_sighash_v2(&tx, 0, &script_pubkey).unwrap();
    let sig = kp.sign(&sighash);
    let mut script_sig = Vec::new();
    push_data(&mut script_sig, &sig);
    push_data(&mut script_sig, &pubkey);
    tx.inputs[0].script_sig = script_sig;

    validate_transaction_v2(&tx, &chain).unwrap();

    let out_op = OutPoint {
        txid: tx.txid(),
        vout: 0,
    };
    let out_entry = UtxoEntryV2 {
        commitment: *c_out.0.as_bytes(),
        script_pubkey,
        encrypted_amount: vec![],
        ephemeral_pubkey: [0u8; 32],
        coinbase: false,
        height: 1,
    };

    chain
        .utxo_store_v2
        .apply_block(&[(out_op, out_entry)], &[in_op])
        .unwrap();
    assert!(chain.utxo_store_v2.get_utxo(&out_op).unwrap().is_some());
    assert!(chain.utxo_store_v2.get_utxo(&in_op).unwrap().is_none());

    chain
        .utxo_store_v2
        .revert_block(&[out_op], &[(in_op, in_entry)])
        .unwrap();
    assert!(chain.utxo_store_v2.get_utxo(&out_op).unwrap().is_none());
    assert!(chain.utxo_store_v2.get_utxo(&in_op).unwrap().is_some());
}

#[test]
fn test_mixed_block() {
    let coinbase = Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig: vec![0x01, 0x01],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value: 50 * 100_000_000,
            script_pubkey: vec![0x51],
        }],
        witnesses: vec![],
        locktime: 0,
    };
    let payment = Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [1u8; 32],
            prev_index: 0,
            script_sig: vec![],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value: 10,
            script_pubkey: vec![0x51],
        }],
        witnesses: vec![],
        locktime: 0,
    };
    let v2 = TransactionV2 {
        version: TX_VERSION_V2,
        inputs: vec![TxInputV2 {
            prev_txid: [2u8; 32],
            prev_index: 0,
            script_sig: vec![],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutputV2 {
            commitment: commit(7, &BlindingFactor([2u8; 32])),
            range_proof: RangeProof(vec![1, 2]),
            script_pubkey: vec![],
            encrypted_amount: vec![1],
            ephemeral_pubkey: [0u8; 32],
        }],
        fee: 0,
        locktime: 0,
    };

    let txs = vec![TxKind::V1(coinbase), TxKind::V1(payment), TxKind::V2(v2)];

    assert!(txs[0].is_coinbase());
    assert!(!txs[1].is_coinbase());
    assert!(!txs[2].is_coinbase());

    let weight: u64 = txs.iter().map(|t| t.weight()).sum();
    assert!(weight > 0);

    let txids: Vec<_> = txs.iter().map(|t| t.txid()).collect();
    assert_eq!(txids.len(), 3);
    let root = compute_merkle_root(&txids);
    assert_ne!(root, [0u8; 32]);

    for t in &txs {
        match t {
            TxKind::V1(x) => x.check_structure().unwrap(),
            TxKind::V2(x) => x.check_structure().unwrap(),
        }
    }
}

#[test]
fn test_v2_intrablock_double_spend_rejected() {
    let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();

    let kp = DilithiumKeypair::generate();
    let pubkey = kp.verifying_key_bytes();
    let script_pubkey = p2dl_script(&pubkey);

    let v_in = 1000u64;
    let fee = 100u64;
    let v_out = v_in - fee;
    let blind = BlindingFactor([5u8; 32]);

    let c_in = commit(v_in, &blind);
    let in_op = OutPoint {
        txid: [9u8; 32],
        vout: 0,
    };
    let in_entry = UtxoEntryV2 {
        commitment: *c_in.0.as_bytes(),
        script_pubkey: script_pubkey.clone(),
        encrypted_amount: vec![],
        ephemeral_pubkey: [0u8; 32],
        coinbase: false,
        height: 0,
    };
    chain.utxo_store_v2.put_utxo(&in_op, &in_entry).unwrap();

    let c_out = commit(v_out, &blind);
    let proof = generate_range_proof(v_out, &blind).unwrap();

    let mut tx = TransactionV2 {
        version: TX_VERSION_V2,
        inputs: vec![TxInputV2 {
            prev_txid: in_op.txid,
            prev_index: in_op.vout,
            script_sig: vec![],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutputV2 {
            commitment: c_out.clone(),
            range_proof: proof,
            script_pubkey: script_pubkey.clone(),
            encrypted_amount: vec![],
            ephemeral_pubkey: [0u8; 32],
        }],
        fee,
        locktime: 0,
    };

    let sighash = compute_sighash_v2(&tx, 0, &script_pubkey).unwrap();
    let sig = kp.sign(&sighash);
    let mut script_sig = Vec::new();
    push_data(&mut script_sig, &sig);
    push_data(&mut script_sig, &pubkey);
    tx.inputs[0].script_sig = script_sig;

    validate_transaction_v2(&tx, &chain).unwrap();

    let header = BlockHeader {
        version: 1,
        prev_block_hash: [0u8; 32],
        merkle_root: [0u8; 32],
        timestamp: 0,
        n_bits: 0,
        nonce: 0,
    };
    let block = Block {
        header,
        transactions: vec![TxKind::V2(tx.clone()), TxKind::V2(tx)],
    };

    let res = chain.collect_v2_utxo_changes(&block, 1);
    assert!(matches!(
        res,
        Err(ChainError::UtxoError(UtxoError::UtxoAlreadySpent))
    ));
}

#[test]
fn test_encrypted_amount_decode_cap() {
    let out = TxOutputV2 {
        commitment: commit(1, &BlindingFactor([1u8; 32])),
        range_proof: RangeProof(vec![1, 2, 3]),
        script_pubkey: vec![0xaa],
        encrypted_amount: vec![0u8; 81],
        ephemeral_pubkey: [0u8; 32],
    };

    let enc = out.encode();
    let res = TxOutputV2::decode(&enc);
    assert!(matches!(res, Err(DecodeError::InvalidData)));
}
