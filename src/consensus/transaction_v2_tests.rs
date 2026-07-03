use crate::consensus::block::{Block, BlockHeader};
use crate::consensus::chain::{ChainError, ChainState};
use crate::consensus::merkle::compute_merkle_root;
use crate::consensus::params::Network;
use crate::consensus::transaction::{
    BlindingFactor, RangeProof, Transaction, TransactionV2, TxInput, TxInputV2, TxKind, TxOutput,
    TxOutputV2, TX_VERSION_V2,
};
use crate::consensus::utxo::{OutPoint, UtxoEntry, UtxoEntryV2, UtxoError};
use crate::consensus::validation::validate_transaction_v2;
use crate::crypto::commitments::{
    commit, generate_range_proof, verify_balance, verify_range_proof,
};
use crate::primitives::serialize::{Decode, DecodeError, Encode};
use crate::script::sighash::{compute_sighash, compute_sighash_v2};
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

#[test]
fn test_v1_intrablock_double_spend_rejected() {
    let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();

    let kp = DilithiumKeypair::generate();
    let pubkey = kp.verifying_key_bytes();
    let script_pubkey = p2dl_script(&pubkey);

    let utxo_output = TxOutput {
        value: 1000,
        script_pubkey: script_pubkey.clone(),
    };
    let in_op = OutPoint {
        txid: [7u8; 32],
        vout: 0,
    };
    chain
        .utxo_store
        .put_utxo(
            &in_op,
            &UtxoEntry {
                output: utxo_output.clone(),
                coinbase: false,
                height: 0,
            },
        )
        .unwrap();

    let mut spend = Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: in_op.txid,
            prev_index: in_op.vout,
            script_sig: vec![],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value: 900,
            script_pubkey: script_pubkey.clone(),
        }],
        witnesses: vec![],
        locktime: 0,
    };

    let sighash = compute_sighash(&spend, 0, std::slice::from_ref(&utxo_output)).unwrap();
    let sig = kp.sign(&sighash);
    let mut script_sig = Vec::new();
    push_data(&mut script_sig, &sig);
    push_data(&mut script_sig, &pubkey);
    spend.inputs[0].script_sig = script_sig;

    let coinbase = Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig: vec![0x01, 0x01],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value: 50,
            script_pubkey: vec![0x51],
        }],
        witnesses: vec![],
        locktime: 0,
    };

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
        transactions: vec![
            TxKind::V1(coinbase),
            TxKind::V1(spend.clone()),
            TxKind::V1(spend),
        ],
    };

    let res = chain.apply_block_to_utxo(&block, 1);
    assert!(matches!(
        res,
        Err(ChainError::UtxoError(UtxoError::UtxoAlreadySpent))
    ));
}

fn build_valid_v2(
    kp: &DilithiumKeypair,
    in_op: OutPoint,
    v_in: u64,
    fee: u64,
) -> (TransactionV2, UtxoEntryV2) {
    let pubkey = kp.verifying_key_bytes();
    let script_pubkey = p2dl_script(&pubkey);
    let v_out = v_in - fee;
    let blind = BlindingFactor([5u8; 32]);

    let c_in = commit(v_in, &blind);
    let in_entry = UtxoEntryV2 {
        commitment: *c_in.0.as_bytes(),
        script_pubkey: script_pubkey.clone(),
        encrypted_amount: vec![],
        ephemeral_pubkey: [0u8; 32],
        coinbase: false,
        height: 0,
    };

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
            commitment: c_out,
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

    (tx, in_entry)
}

fn coinbase_v1(height: u32) -> Transaction {
    let mut script_sig = Vec::new();
    script_sig.push(4);
    script_sig.extend_from_slice(&height.to_le_bytes());
    Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig,
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value: crate::consensus::validation::calculate_subsidy(height),
            script_pubkey: vec![0x51],
        }],
        witnesses: vec![],
        locktime: 0,
    }
}

#[test]
fn test_v2_mempool_admit() {
    use crate::network::mempool::NetworkMempool;
    use std::sync::{Arc, RwLock};

    let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let kp = DilithiumKeypair::generate();
    let in_op = OutPoint {
        txid: [9u8; 32],
        vout: 0,
    };
    let (tx, in_entry) = build_valid_v2(&kp, in_op, 1000, 100);
    chain.utxo_store_v2.put_utxo(&in_op, &in_entry).unwrap();

    let txid = tx.txid();
    let chain = Arc::new(RwLock::new(chain));
    let mempool = NetworkMempool::new(chain);

    let admitted = mempool.add_transaction(TxKind::V2(tx)).unwrap();
    assert!(admitted);
    assert!(mempool.has_transaction(&txid));
    assert_eq!(mempool.size(), 1);
}

#[test]
fn test_v2_miner_includes_v2() {
    use crate::consensus::block::create_genesis_block;
    use crate::mining::miner::Miner;

    let mut chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let genesis = create_genesis_block(0x207f_ffff);
    let g_work = genesis.header.work();
    chain.seed_block_for_test(genesis, 0, g_work, true);

    let kp = DilithiumKeypair::generate();
    let in_op = OutPoint {
        txid: [9u8; 32],
        vout: 0,
    };
    let (tx, in_entry) = build_valid_v2(&kp, in_op, 1000, 100);
    chain.utxo_store_v2.put_utxo(&in_op, &in_entry).unwrap();
    let v2_txid = tx.txid();

    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);
    let template = miner
        .build_template(&chain, vec![TxKind::V2(tx)])
        .expect("template ok");

    assert!(template
        .transactions
        .iter()
        .any(|t| matches!(t, TxKind::V2(v2) if v2.txid() == v2_txid)));
}

#[test]
fn test_v2_reorg_unblocked() {
    use crate::consensus::block::create_genesis_block;

    let mut chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let genesis = create_genesis_block(0x207f_ffff);
    let g_hash = genesis.hash();
    let g_work = genesis.header.work();
    chain.seed_block_for_test(genesis, 0, g_work, true);

    let kp = DilithiumKeypair::generate();
    let in_op = OutPoint {
        txid: [9u8; 32],
        vout: 0,
    };
    let (v2_tx, in_entry) = build_valid_v2(&kp, in_op, 1000, 100);
    let out_op = OutPoint {
        txid: v2_tx.txid(),
        vout: 0,
    };
    let out_entry = UtxoEntryV2 {
        commitment: *v2_tx.outputs[0].commitment.0.as_bytes(),
        script_pubkey: v2_tx.outputs[0].script_pubkey.clone(),
        encrypted_amount: v2_tx.outputs[0].encrypted_amount.clone(),
        ephemeral_pubkey: v2_tx.outputs[0].ephemeral_pubkey,
        coinbase: false,
        height: 1,
    };

    let cb_a = coinbase_v1(1);
    let cb_a_op = OutPoint {
        txid: cb_a.txid(),
        vout: 0,
    };
    let cb_a_entry = UtxoEntry {
        output: cb_a.outputs[0].clone(),
        coinbase: true,
        height: 1,
    };
    let a_txs = vec![TxKind::V1(cb_a), TxKind::V2(v2_tx)];
    let a_txids: Vec<_> = a_txs.iter().map(|t| t.txid()).collect();
    let block_a = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: g_hash,
            merkle_root: compute_merkle_root(&a_txids),
            timestamp: 1_700_000_100,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: a_txs,
    };
    let a_hash = block_a.hash();
    let cum_a = g_work + block_a.header.work();
    chain.seed_block_for_test(block_a, 1, cum_a, true);

    chain
        .utxo_store
        .apply_block(&[(cb_a_op, cb_a_entry)], &[])
        .unwrap();
    chain.utxo_store.store_undo_data(&a_hash, &[]).unwrap();
    chain.utxo_store_v2.put_utxo(&out_op, &out_entry).unwrap();
    chain
        .utxo_store_v2
        .store_undo_data(&a_hash, &[(in_op, in_entry.clone())])
        .unwrap();

    let cb_b = coinbase_v1(1);
    let b_txids = vec![cb_b.txid()];
    let block_b = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: g_hash,
            merkle_root: compute_merkle_root(&b_txids),
            timestamp: 1_700_000_050,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb_b)],
    };
    let b_hash = block_b.hash();
    let cum_b = g_work + block_b.header.work();
    chain.seed_block_for_test(block_b, 1, cum_b, false);

    let cb_b2 = coinbase_v1(2);
    let b2_txids = vec![cb_b2.txid()];
    let block_b2 = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: b_hash,
            merkle_root: compute_merkle_root(&b2_txids),
            timestamp: 1_700_000_060,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb_b2)],
    };
    let b2_hash = block_b2.hash();
    let cum_b2 = cum_b + block_b2.header.work();
    chain.seed_block_for_test(block_b2, 2, cum_b2, false);

    assert!(chain.utxo_store_v2.get_utxo(&out_op).unwrap().is_some());
    assert!(chain.utxo_store_v2.get_utxo(&in_op).unwrap().is_none());

    chain.reorganize(&a_hash, &b2_hash).unwrap();

    assert!(chain.utxo_store_v2.get_utxo(&out_op).unwrap().is_none());
    let restored = chain.utxo_store_v2.get_utxo(&in_op).unwrap();
    assert_eq!(restored, Some(in_entry));
}

#[test]
fn test_block_message_v2_roundtrip() {
    use crate::network::protocol::BlockMessage;

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

    let v2 = TxKind::V2(TransactionV2 {
        version: TX_VERSION_V2,
        inputs: vec![TxInputV2 {
            prev_txid: [2u8; 32],
            prev_index: 1,
            script_sig: vec![7, 8, 9],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutputV2 {
            commitment: commit(500, &BlindingFactor([3u8; 32])),
            range_proof: RangeProof(vec![1, 2, 3, 4]),
            script_pubkey: vec![0xaa, 0x20],
            encrypted_amount: vec![9, 9, 9],
            ephemeral_pubkey: [4u8; 32],
        }],
        fee: 7,
        locktime: 0,
    });

    let msg = BlockMessage {
        header: BlockHeader {
            version: 1,
            prev_block_hash: [5u8; 32],
            merkle_root: [6u8; 32],
            timestamp: 123,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![v1, v2],
    };

    let encoded = msg.encode();
    assert_eq!(encoded.len(), msg.encoded_size());

    let (decoded, consumed) = BlockMessage::decode(&encoded).unwrap();
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded.transactions.len(), 2);
    assert_eq!(decoded, msg);
    assert!(matches!(decoded.transactions[0], TxKind::V1(_)));
    assert!(matches!(decoded.transactions[1], TxKind::V2(_)));
}

#[test]
fn test_v2_fee_claimed_in_coinbase() {
    use crate::consensus::validation::{
        calculate_subsidy, validate_coinbase_reward, ValidationError,
    };

    let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let kp = DilithiumKeypair::generate();
    let in_op = OutPoint {
        txid: [9u8; 32],
        vout: 0,
    };
    let (v2_tx, in_entry) = build_valid_v2(&kp, in_op, 3000, 1000);
    chain.utxo_store_v2.put_utxo(&in_op, &in_entry).unwrap();

    let height: u32 = 1;
    let subsidy = calculate_subsidy(height);

    let mut coinbase = coinbase_v1(height);
    coinbase.outputs[0].value = subsidy + 1000;

    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: [0u8; 32],
            merkle_root: [0u8; 32],
            timestamp: 0,
            n_bits: 0,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(coinbase.clone()), TxKind::V2(v2_tx)],
    };

    let (_, _, _, v1_fees) = chain.apply_block_to_utxo(&block, height as u64).unwrap();
    let v2_changes = chain
        .collect_v2_utxo_changes(&block, height as u64)
        .unwrap();
    let block_fees = v1_fees + v2_changes.fees;
    assert_eq!(block_fees, 1000);

    assert!(validate_coinbase_reward(&coinbase, block_fees, height).is_ok());

    let mut over = coinbase_v1(height);
    over.outputs[0].value = subsidy + 1001;
    assert_eq!(
        validate_coinbase_reward(&over, block_fees, height),
        Err(ValidationError::CoinbaseRewardTooHigh)
    );

    assert_eq!(
        validate_coinbase_reward(&coinbase, 0, height),
        Err(ValidationError::CoinbaseRewardTooHigh)
    );
}

#[test]
fn test_add_block_atomic_all_cfs() {
    use crate::storage::ChainTip;

    let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();

    let pre_v1_op = OutPoint {
        txid: [1u8; 32],
        vout: 0,
    };
    let pre_v1_entry = UtxoEntry {
        output: TxOutput {
            value: 5000,
            script_pubkey: vec![0x51],
        },
        coinbase: false,
        height: 0,
    };
    chain
        .utxo_store
        .put_utxo(&pre_v1_op, &pre_v1_entry)
        .unwrap();

    let pre_v2_op = OutPoint {
        txid: [2u8; 32],
        vout: 0,
    };
    let pre_v2_entry = UtxoEntryV2 {
        commitment: [3u8; 32],
        script_pubkey: vec![0xaa, 0x20],
        encrypted_amount: vec![],
        ephemeral_pubkey: [4u8; 32],
        coinbase: false,
        height: 0,
    };
    chain
        .utxo_store_v2
        .put_utxo(&pre_v2_op, &pre_v2_entry)
        .unwrap();

    let new_v1_op = OutPoint {
        txid: [10u8; 32],
        vout: 0,
    };
    let new_v1_entry = UtxoEntry {
        output: TxOutput {
            value: 4000,
            script_pubkey: vec![0x51],
        },
        coinbase: false,
        height: 1,
    };
    let new_v2_op = OutPoint {
        txid: [11u8; 32],
        vout: 0,
    };
    let new_v2_entry = UtxoEntryV2 {
        commitment: [12u8; 32],
        script_pubkey: vec![0xaa, 0x20],
        encrypted_amount: vec![1, 2, 3],
        ephemeral_pubkey: [13u8; 32],
        coinbase: false,
        height: 1,
    };

    let created_v1 = vec![(new_v1_op, new_v1_entry.clone())];
    let spent_v1 = vec![pre_v1_op];
    let undo_v1 = vec![(pre_v1_op, pre_v1_entry.clone())];
    let created_v2 = vec![(new_v2_op, new_v2_entry.clone())];
    let spent_v2 = vec![pre_v2_op];
    let undo_v2 = vec![(pre_v2_op, pre_v2_entry.clone())];

    let cb = coinbase_v1(1);
    let txids = vec![cb.txid()];
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: [0u8; 32],
            merkle_root: compute_merkle_root(&txids),
            timestamp: 1_700_000_000,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb)],
    };
    let block_hash = block.hash();
    let cum_work = block.header.work();
    let tip = ChainTip {
        hash: block_hash,
        height: 1,
        cumulative_work: cum_work,
    };

    let mut batch = chain.db.batch();
    chain
        .utxo_store
        .stage_apply_block(&mut batch, &created_v1, &spent_v1)
        .unwrap();
    chain
        .utxo_store
        .stage_undo_data(&mut batch, &block_hash, &undo_v1)
        .unwrap();
    chain
        .utxo_store_v2
        .stage_apply_block(&mut batch, &created_v2, &spent_v2)
        .unwrap();
    chain
        .utxo_store_v2
        .stage_undo_data(&mut batch, &block_hash, &undo_v2)
        .unwrap();
    chain
        .block_store
        .stage_store_block(&mut batch, &block, 1, cum_work)
        .unwrap();
    chain.state_store.stage_set_tip(&mut batch, &tip).unwrap();
    batch.commit().unwrap();

    assert_eq!(
        chain.utxo_store.get_utxo(&new_v1_op).unwrap(),
        Some(new_v1_entry)
    );
    assert_eq!(chain.utxo_store.get_utxo(&pre_v1_op).unwrap(), None);
    assert_eq!(
        chain.utxo_store_v2.get_utxo(&new_v2_op).unwrap(),
        Some(new_v2_entry)
    );
    assert_eq!(chain.utxo_store_v2.get_utxo(&pre_v2_op).unwrap(), None);
    assert_eq!(
        chain.utxo_store.get_undo_data(&block_hash).unwrap(),
        Some(undo_v1)
    );
    assert_eq!(
        chain.utxo_store_v2.get_undo_data(&block_hash).unwrap(),
        Some(undo_v2)
    );
    assert!(chain.block_store.get_block(&block_hash).unwrap().is_some());
    assert!(chain.block_store.get_header(&block_hash).unwrap().is_some());
    assert_eq!(
        chain.block_store.get_hash_by_height(1).unwrap(),
        Some(block_hash)
    );
    let meta = chain
        .block_store
        .get_block_meta(&block_hash)
        .unwrap()
        .unwrap();
    assert_eq!(meta.height, 1);
    assert_eq!(meta.cumulative_work, cum_work);
    assert_eq!(chain.state_store.get_tip().unwrap(), Some(tip));
}

#[test]
fn test_add_block_tip_coupled_to_utxo() {
    use crate::storage::ChainTip;

    let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();

    let pre_op = OutPoint {
        txid: [1u8; 32],
        vout: 0,
    };
    let pre_entry = UtxoEntry {
        output: TxOutput {
            value: 5000,
            script_pubkey: vec![0x51],
        },
        coinbase: false,
        height: 0,
    };
    chain.utxo_store.put_utxo(&pre_op, &pre_entry).unwrap();

    let new_op = OutPoint {
        txid: [10u8; 32],
        vout: 0,
    };
    let new_entry = UtxoEntry {
        output: TxOutput {
            value: 4000,
            script_pubkey: vec![0x51],
        },
        coinbase: false,
        height: 5,
    };

    let cb = coinbase_v1(5);
    let txids = vec![cb.txid()];
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: [7u8; 32],
            merkle_root: compute_merkle_root(&txids),
            timestamp: 1_700_000_500,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb)],
    };
    let block_hash = block.hash();
    let cum_work = block.header.work();
    let tip = ChainTip {
        hash: block_hash,
        height: 5,
        cumulative_work: cum_work,
    };

    let mut batch = chain.db.batch();
    chain
        .utxo_store
        .stage_apply_block(&mut batch, &[(new_op, new_entry.clone())], &[pre_op])
        .unwrap();
    chain
        .utxo_store
        .stage_undo_data(&mut batch, &block_hash, &[(pre_op, pre_entry)])
        .unwrap();
    chain
        .block_store
        .stage_store_block(&mut batch, &block, 5, cum_work)
        .unwrap();
    chain.state_store.stage_set_tip(&mut batch, &tip).unwrap();
    batch.commit().unwrap();

    let stored_tip = chain.state_store.get_tip().unwrap().unwrap();
    assert_eq!(stored_tip.height, 5);
    assert_eq!(stored_tip.hash, block_hash);
    assert_eq!(chain.utxo_store.get_utxo(&pre_op).unwrap(), None);
    assert_eq!(chain.utxo_store.get_utxo(&new_op).unwrap(), Some(new_entry));
}

#[test]
fn test_stage_produces_identical_bytes() {
    use crate::storage::{
        ChainTip, CF_BLOCKS, CF_BLOCK_INDEX, CF_BLOCK_META, CF_CHAIN_STATE, CF_HEADERS, CF_UNDO,
        CF_UNDO_V2, CF_UTXO, CF_UTXO_V2,
    };

    type V1Set = Vec<(OutPoint, UtxoEntry)>;
    type V2Set = Vec<(OutPoint, UtxoEntryV2)>;

    let build_inputs = || -> (V1Set, Vec<OutPoint>, V1Set, V2Set, Vec<OutPoint>, V2Set) {
        let created_v1 = vec![(
            OutPoint {
                txid: [10u8; 32],
                vout: 0,
            },
            UtxoEntry {
                output: TxOutput {
                    value: 4000,
                    script_pubkey: vec![0x51],
                },
                coinbase: false,
                height: 1,
            },
        )];
        let spent_v1 = vec![OutPoint {
            txid: [1u8; 32],
            vout: 0,
        }];
        let undo_v1 = vec![(
            OutPoint {
                txid: [1u8; 32],
                vout: 0,
            },
            UtxoEntry {
                output: TxOutput {
                    value: 5000,
                    script_pubkey: vec![0x51],
                },
                coinbase: false,
                height: 0,
            },
        )];
        let created_v2 = vec![(
            OutPoint {
                txid: [11u8; 32],
                vout: 0,
            },
            UtxoEntryV2 {
                commitment: [12u8; 32],
                script_pubkey: vec![0xaa, 0x20],
                encrypted_amount: vec![1, 2, 3],
                ephemeral_pubkey: [13u8; 32],
                coinbase: false,
                height: 1,
            },
        )];
        let spent_v2 = vec![OutPoint {
            txid: [2u8; 32],
            vout: 0,
        }];
        let undo_v2 = vec![(
            OutPoint {
                txid: [2u8; 32],
                vout: 0,
            },
            UtxoEntryV2 {
                commitment: [3u8; 32],
                script_pubkey: vec![0xaa, 0x20],
                encrypted_amount: vec![],
                ephemeral_pubkey: [4u8; 32],
                coinbase: false,
                height: 0,
            },
        )];
        (created_v1, spent_v1, undo_v1, created_v2, spent_v2, undo_v2)
    };

    let cb = coinbase_v1(1);
    let txids = vec![cb.txid()];
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: [0u8; 32],
            merkle_root: compute_merkle_root(&txids),
            timestamp: 1_700_000_000,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb)],
    };
    let block_hash = block.hash();
    let cum_work = block.header.work();
    let tip = ChainTip {
        hash: block_hash,
        height: 1,
        cumulative_work: cum_work,
    };

    let chain_a = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let (c1, s1, u1, c2, s2, u2) = build_inputs();
    chain_a.utxo_store.apply_block(&c1, &s1).unwrap();
    chain_a
        .utxo_store
        .store_undo_data(&block_hash, &u1)
        .unwrap();
    chain_a.utxo_store_v2.apply_block(&c2, &s2).unwrap();
    chain_a
        .utxo_store_v2
        .store_undo_data(&block_hash, &u2)
        .unwrap();
    chain_a
        .block_store
        .store_block(&block, 1, cum_work)
        .unwrap();
    chain_a.state_store.set_tip(&tip).unwrap();

    let chain_b = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let (c1, s1, u1, c2, s2, u2) = build_inputs();
    let mut batch = chain_b.db.batch();
    chain_b
        .utxo_store
        .stage_apply_block(&mut batch, &c1, &s1)
        .unwrap();
    chain_b
        .utxo_store
        .stage_undo_data(&mut batch, &block_hash, &u1)
        .unwrap();
    chain_b
        .utxo_store_v2
        .stage_apply_block(&mut batch, &c2, &s2)
        .unwrap();
    chain_b
        .utxo_store_v2
        .stage_undo_data(&mut batch, &block_hash, &u2)
        .unwrap();
    chain_b
        .block_store
        .stage_store_block(&mut batch, &block, 1, cum_work)
        .unwrap();
    chain_b.state_store.stage_set_tip(&mut batch, &tip).unwrap();
    batch.commit().unwrap();

    for cf in [
        CF_UTXO,
        CF_UTXO_V2,
        CF_UNDO,
        CF_UNDO_V2,
        CF_BLOCKS,
        CF_HEADERS,
        CF_BLOCK_INDEX,
        CF_BLOCK_META,
        CF_CHAIN_STATE,
    ] {
        let mut a = chain_a.db.iter(cf).unwrap();
        let mut b = chain_b.db.iter(cf).unwrap();
        a.sort();
        b.sort();
        assert_eq!(
            a, b,
            "CF {} differs between committing and staged paths",
            cf
        );
    }
}

struct ReorgFixture {
    chain: ChainState,
    cb_a_op: OutPoint,
    cb_b_op: OutPoint,
    cb_b_entry: UtxoEntry,
    cb_b2_op: OutPoint,
    b_hash: [u8; 32],
    b2_hash: [u8; 32],
    cum_b2: crate::consensus::block::U256,
}

fn reorg_fixture() -> ReorgFixture {
    use crate::consensus::block::create_genesis_block;

    let mut chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let genesis = create_genesis_block(0x207f_ffff);
    let g_hash = genesis.hash();
    let g_work = genesis.header.work();
    chain.seed_block_for_test(genesis, 0, g_work, true);

    let mut cb_a = coinbase_v1(1);
    cb_a.outputs[0].script_pubkey = vec![0xa1];
    let cb_a_op = OutPoint {
        txid: cb_a.txid(),
        vout: 0,
    };
    let cb_a_entry = UtxoEntry {
        output: cb_a.outputs[0].clone(),
        coinbase: true,
        height: 1,
    };
    let a_txids = vec![cb_a.txid()];
    let block_a = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: g_hash,
            merkle_root: compute_merkle_root(&a_txids),
            timestamp: 1_700_000_100,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb_a)],
    };
    let a_hash = block_a.hash();
    let cum_a = g_work + block_a.header.work();
    chain.seed_block_for_test(block_a, 1, cum_a, true);
    chain
        .utxo_store
        .apply_block(&[(cb_a_op, cb_a_entry)], &[])
        .unwrap();
    chain.utxo_store.store_undo_data(&a_hash, &[]).unwrap();

    let mut cb_b = coinbase_v1(1);
    cb_b.outputs[0].script_pubkey = vec![0xb1];
    let cb_b_op = OutPoint {
        txid: cb_b.txid(),
        vout: 0,
    };
    let cb_b_entry = UtxoEntry {
        output: cb_b.outputs[0].clone(),
        coinbase: true,
        height: 1,
    };
    let b_txids = vec![cb_b.txid()];
    let block_b = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: g_hash,
            merkle_root: compute_merkle_root(&b_txids),
            timestamp: 1_700_000_050,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb_b)],
    };
    let b_hash = block_b.hash();
    let cum_b = g_work + block_b.header.work();
    chain.seed_block_for_test(block_b, 1, cum_b, false);

    let cb_b2 = coinbase_v1(2);
    let cb_b2_op = OutPoint {
        txid: cb_b2.txid(),
        vout: 0,
    };
    let b2_txids = vec![cb_b2.txid()];
    let block_b2 = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: b_hash,
            merkle_root: compute_merkle_root(&b2_txids),
            timestamp: 1_700_000_060,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb_b2)],
    };
    let b2_hash = block_b2.hash();
    let cum_b2 = cum_b + block_b2.header.work();
    chain.seed_block_for_test(block_b2, 2, cum_b2, false);

    chain.reorganize(&a_hash, &b2_hash).unwrap();

    ReorgFixture {
        chain,
        cb_a_op,
        cb_b_op,
        cb_b_entry,
        cb_b2_op,
        b_hash,
        b2_hash,
        cum_b2,
    }
}

#[test]
fn test_reorg_single_batch() {
    let f = reorg_fixture();

    assert_eq!(f.chain.utxo_store.get_utxo(&f.cb_a_op).unwrap(), None);
    assert!(f.chain.utxo_store.get_utxo(&f.cb_b_op).unwrap().is_some());
    assert!(f.chain.utxo_store.get_utxo(&f.cb_b2_op).unwrap().is_some());

    let undo_b = f
        .chain
        .utxo_store
        .get_undo_data(&f.b_hash)
        .unwrap()
        .unwrap();
    assert!(undo_b.is_empty());
    let undo_b2 = f
        .chain
        .utxo_store
        .get_undo_data(&f.b2_hash)
        .unwrap()
        .unwrap();
    assert!(undo_b2.is_empty());

    assert!(f.chain.block_store.get_block(&f.b_hash).unwrap().is_some());
    assert!(f.chain.block_store.get_block(&f.b2_hash).unwrap().is_some());
    assert_eq!(
        f.chain.block_store.get_hash_by_height(1).unwrap(),
        Some(f.b_hash)
    );
    assert_eq!(
        f.chain.block_store.get_hash_by_height(2).unwrap(),
        Some(f.b2_hash)
    );

    let tip = f.chain.state_store.get_tip().unwrap().unwrap();
    assert_eq!(tip.hash, f.b2_hash);
    assert_eq!(tip.height, 2);
    assert_eq!(tip.cumulative_work, f.cum_b2);
}

#[test]
fn test_reorg_encoder_unified() {
    use crate::storage::CF_UTXO;

    let f = reorg_fixture();

    let mut key = Vec::with_capacity(36);
    key.extend_from_slice(&f.cb_b_op.txid);
    key.extend_from_slice(&f.cb_b_op.vout.to_le_bytes());

    let reorg_bytes = f.chain.db.get(CF_UTXO, &key).unwrap();
    assert!(reorg_bytes.is_some());

    let chain2 = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let mut batch = chain2.db.batch();
    chain2
        .utxo_store
        .stage_apply_block(&mut batch, &[(f.cb_b_op, f.cb_b_entry.clone())], &[])
        .unwrap();
    batch.commit().unwrap();
    let ref_bytes = chain2.db.get(CF_UTXO, &key).unwrap();

    assert_eq!(reorg_bytes, ref_bytes);
}

#[test]
fn test_turnstile_genesis_zero() {
    let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    assert_eq!(chain.state_store.get_pool_value().unwrap(), 0);
}

#[test]
fn test_turnstile_entry_credits_x_minus_fee() {
    let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let x: u64 = 1_000_000;
    let f: u64 = 10_000;
    let v1_in = OutPoint {
        txid: [7u8; 32],
        vout: 0,
    };
    let v1_entry = UtxoEntry {
        output: TxOutput {
            value: x,
            script_pubkey: vec![0x51],
        },
        coinbase: false,
        height: 0,
    };
    let spent_v1_entries = vec![(v1_in, v1_entry)];

    let delta = ChainState::pool_delta(&spent_v1_entries, f);
    assert_eq!(delta, (x - f) as i128);

    let block_hash = [1u8; 32];
    let pool_value = chain.state_store.get_pool_value().unwrap();
    let new_pool = (pool_value as i128 + delta) as u128;
    let mut batch = chain.db.batch();
    chain
        .state_store
        .stage_pool_update(&mut batch, &block_hash, delta, new_pool)
        .unwrap();
    batch.commit().unwrap();

    assert_eq!(chain.state_store.get_pool_value().unwrap(), (x - f) as u128);
    assert_eq!(
        chain.state_store.get_pool_delta(&block_hash).unwrap(),
        Some((x - f) as i128)
    );
}

#[test]
fn test_turnstile_v2v2_spend_only_deducts_fee() {
    let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let f: u64 = 10_000;

    let delta = ChainState::pool_delta(&[], f);
    assert_eq!(delta, -(f as i128));

    let p: u128 = 1_000_000;
    let mut batch = chain.db.batch();
    chain.state_store.stage_pool_value(&mut batch, p).unwrap();
    batch.commit().unwrap();
    assert_eq!(chain.state_store.get_pool_value().unwrap(), p);

    let block_hash = [2u8; 32];
    let new_pool = (p as i128 + delta) as u128;
    let mut batch = chain.db.batch();
    chain
        .state_store
        .stage_pool_update(&mut batch, &block_hash, delta, new_pool)
        .unwrap();
    batch.commit().unwrap();

    assert_eq!(chain.state_store.get_pool_value().unwrap(), p - f as u128);
}

#[test]
fn test_turnstile_reorg_reverses_delta() {
    use crate::consensus::block::create_genesis_block;

    let mut chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let genesis = create_genesis_block(0x207f_ffff);
    let g_hash = genesis.hash();
    let g_work = genesis.header.work();
    chain.seed_block_for_test(genesis, 0, g_work, true);

    let cb_a = coinbase_v1(1);
    let a_txids = vec![cb_a.txid()];
    let block_a = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: g_hash,
            merkle_root: compute_merkle_root(&a_txids),
            timestamp: 1_700_000_100,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb_a)],
    };
    let a_hash = block_a.hash();
    let cum_a = g_work + block_a.header.work();
    chain.seed_block_for_test(block_a, 1, cum_a, true);
    chain.utxo_store.store_undo_data(&a_hash, &[]).unwrap();

    let d_a: i128 = 500_000;
    let mut batch = chain.db.batch();
    chain
        .state_store
        .stage_pool_update(&mut batch, &a_hash, d_a, d_a as u128)
        .unwrap();
    batch.commit().unwrap();
    assert_eq!(chain.state_store.get_pool_value().unwrap(), d_a as u128);

    let mut cb_b = coinbase_v1(1);
    cb_b.outputs[0].script_pubkey = vec![0xb1];
    let b_txids = vec![cb_b.txid()];
    let block_b = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: g_hash,
            merkle_root: compute_merkle_root(&b_txids),
            timestamp: 1_700_000_050,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb_b)],
    };
    let b_hash = block_b.hash();
    let cum_b = g_work + block_b.header.work();
    chain.seed_block_for_test(block_b, 1, cum_b, false);

    let cb_b2 = coinbase_v1(2);
    let b2_txids = vec![cb_b2.txid()];
    let block_b2 = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: b_hash,
            merkle_root: compute_merkle_root(&b2_txids),
            timestamp: 1_700_000_060,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb_b2)],
    };
    let b2_hash = block_b2.hash();
    let cum_b2 = cum_b + block_b2.header.work();
    chain.seed_block_for_test(block_b2, 2, cum_b2, false);

    chain.reorganize(&a_hash, &b2_hash).unwrap();

    assert_eq!(chain.state_store.get_pool_value().unwrap(), 0);
    assert_eq!(chain.state_store.get_pool_delta(&a_hash).unwrap(), None);
    assert_eq!(chain.state_store.get_pool_delta(&b_hash).unwrap(), Some(0));
    assert_eq!(chain.state_store.get_pool_delta(&b2_hash).unwrap(), Some(0));
}

#[test]
fn test_turnstile_underflow_rejects_block() {
    use crate::consensus::block::create_genesis_block;
    use crate::consensus::validation::{calculate_subsidy, ValidationError};

    let mut chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
    let genesis = create_genesis_block(0x207f_ffff);
    let g_hash = genesis.hash();
    let g_work = genesis.header.work();
    chain.seed_block_for_test(genesis, 0, g_work, true);

    let kp = DilithiumKeypair::generate();
    let v_in: u64 = 1_000_000;
    let fee: u64 = 50_000;
    let in_op = OutPoint {
        txid: [9u8; 32],
        vout: 0,
    };
    let (v2_tx, in_entry) = build_valid_v2(&kp, in_op, v_in, fee);
    chain.utxo_store_v2.put_utxo(&in_op, &in_entry).unwrap();

    let cb_a = coinbase_v1(1);
    let a_txids = vec![cb_a.txid()];
    let block_a = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: g_hash,
            merkle_root: compute_merkle_root(&a_txids),
            timestamp: 1_700_000_100,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb_a)],
    };
    let a_hash = block_a.hash();
    let cum_a = g_work + block_a.header.work();
    chain.seed_block_for_test(block_a, 1, cum_a, true);
    chain.utxo_store.store_undo_data(&a_hash, &[]).unwrap();

    let mut cb_b = coinbase_v1(1);
    cb_b.outputs[0].value = calculate_subsidy(1) + fee;
    cb_b.outputs[0].script_pubkey = vec![0xb1];
    let b_txs = vec![TxKind::V1(cb_b), TxKind::V2(v2_tx)];
    let b_txids: Vec<_> = b_txs.iter().map(|t| t.txid()).collect();
    let block_b = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: g_hash,
            merkle_root: compute_merkle_root(&b_txids),
            timestamp: 1_700_000_050,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: b_txs,
    };
    let b_hash = block_b.hash();
    let cum_b = g_work + block_b.header.work();
    chain.seed_block_for_test(block_b, 1, cum_b, false);

    let cb_b2 = coinbase_v1(2);
    let b2_txids = vec![cb_b2.txid()];
    let block_b2 = Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: b_hash,
            merkle_root: compute_merkle_root(&b2_txids),
            timestamp: 1_700_000_060,
            n_bits: 0x207f_ffff,
            nonce: 0,
        },
        transactions: vec![TxKind::V1(cb_b2)],
    };
    let b2_hash = block_b2.hash();
    let cum_b2 = cum_b + block_b2.header.work();
    chain.seed_block_for_test(block_b2, 2, cum_b2, false);

    assert_eq!(chain.state_store.get_pool_value().unwrap(), 0);
    let res = chain.reorganize(&a_hash, &b2_hash);
    assert_eq!(
        res,
        Err(ChainError::InvalidBlock(
            ValidationError::TurnstileUnderflow
        ))
    );
}

#[test]
fn test_rpc_kind_labels_and_serialization() {
    let coinbase = TxKind::V1(Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
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
    assert!(coinbase.is_coinbase());
    assert_eq!(coinbase.rpc_kind(), "coinbase");

    let transfer = TxKind::V1(Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [7u8; 32],
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
    assert!(!transfer.is_coinbase());
    assert_eq!(transfer.rpc_kind(), "transfer");

    let confidential = TxKind::V2(TransactionV2 {
        version: TX_VERSION_V2,
        inputs: vec![TxInputV2 {
            prev_txid: [9u8; 32],
            prev_index: 1,
            script_sig: vec![],
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
    });
    assert!(!confidential.is_coinbase());
    assert_eq!(confidential.rpc_kind(), "confidential");

    let mempool_tx = crate::rpc::methods::MempoolTx {
        txid: "aa".to_string(),
        size: 10,
        vsize: 10,
        vin_count: 1,
        vout_count: 1,
        output_value: 0,
        fee: Some(11),
        kind: confidential.rpc_kind().to_string(),
    };
    assert!(serde_json::to_string(&mempool_tx)
        .unwrap()
        .contains("\"kind\":\"confidential\""));

    let verbose_coinbase = crate::rpc::methods::VerboseTx {
        txid: "bb".to_string(),
        is_coinbase: true,
        kind: "coinbase".to_string(),
        vin: vec![],
        vout: vec![],
    };
    assert!(serde_json::to_string(&verbose_coinbase)
        .unwrap()
        .contains("\"kind\":\"coinbase\""));

    let verbose_transfer = crate::rpc::methods::VerboseTx {
        txid: "cc".to_string(),
        is_coinbase: false,
        kind: "transfer".to_string(),
        vin: vec![],
        vout: vec![],
    };
    assert!(serde_json::to_string(&verbose_transfer)
        .unwrap()
        .contains("\"kind\":\"transfer\""));
}
