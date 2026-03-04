//! Tests for merkle tree construction

use ferrous_node::consensus::merkle::{compute_merkle_root, compute_witness_merkle_root};
use ferrous_node::primitives::hash::Hash256;

#[test]
fn test_empty_merkle_root() {
    let txids: Vec<Hash256> = vec![];
    let root = compute_merkle_root(&txids);
    assert_eq!(root, [0u8; 32]);
}

#[test]
fn test_single_tx_merkle_root() {
    let txid = [1u8; 32];
    let txids = vec![txid];
    let root = compute_merkle_root(&txids);
    assert_eq!(root, txid);
}

#[test]
fn test_two_tx_merkle_root() {
    let txid1 = [1u8; 32];
    let txid2 = [2u8; 32];
    let txids = vec![txid1, txid2];
    let root = compute_merkle_root(&txids);

    // Expected: sha256d(txid1 || txid2)
    let mut data = Vec::new();
    data.extend_from_slice(&txid1);
    data.extend_from_slice(&txid2);
    let expected = ferrous_node::primitives::hash::sha256d(&data);

    assert_eq!(root, expected);
}

#[test]
fn test_three_tx_merkle_root() {
    let txid1 = [1u8; 32];
    let txid2 = [2u8; 32];
    let txid3 = [3u8; 32];
    let txids = vec![txid1, txid2, txid3];
    let root = compute_merkle_root(&txids);

    // Level 1: sha256d(txid1 || txid2), sha256d(txid3 || txid3)
    let mut data1 = Vec::new();
    data1.extend_from_slice(&txid1);
    data1.extend_from_slice(&txid2);
    let hash1 = ferrous_node::primitives::hash::sha256d(&data1);

    let mut data2 = Vec::new();
    data2.extend_from_slice(&txid3);
    data2.extend_from_slice(&txid3);
    let hash2 = ferrous_node::primitives::hash::sha256d(&data2);

    // Level 2: sha256d(hash1 || hash2)
    let mut data3 = Vec::new();
    data3.extend_from_slice(&hash1);
    data3.extend_from_slice(&hash2);
    let expected = ferrous_node::primitives::hash::sha256d(&data3);

    assert_eq!(root, expected);
}

#[test]
fn test_four_tx_merkle_root() {
    let txid1 = [1u8; 32];
    let txid2 = [2u8; 32];
    let txid3 = [3u8; 32];
    let txid4 = [4u8; 32];
    let txids = vec![txid1, txid2, txid3, txid4];
    let root = compute_merkle_root(&txids);

    // Level 1: sha256d(txid1 || txid2), sha256d(txid3 || txid4)
    let mut data1 = Vec::new();
    data1.extend_from_slice(&txid1);
    data1.extend_from_slice(&txid2);
    let hash1 = ferrous_node::primitives::hash::sha256d(&data1);

    let mut data2 = Vec::new();
    data2.extend_from_slice(&txid3);
    data2.extend_from_slice(&txid4);
    let hash2 = ferrous_node::primitives::hash::sha256d(&data2);

    // Level 2: sha256d(hash1 || hash2)
    let mut data3 = Vec::new();
    data3.extend_from_slice(&hash1);
    data3.extend_from_slice(&hash2);
    let expected = ferrous_node::primitives::hash::sha256d(&data3);

    assert_eq!(root, expected);
}

#[test]
fn test_five_tx_merkle_root() {
    let txid1 = [1u8; 32];
    let txid2 = [2u8; 32];
    let txid3 = [3u8; 32];
    let txid4 = [4u8; 32];
    let txid5 = [5u8; 32];
    let txids = vec![txid1, txid2, txid3, txid4, txid5];
    let root = compute_merkle_root(&txids);

    // Level 1: sha256d(txid1 || txid2), sha256d(txid3 || txid4), sha256d(txid5 || txid5)
    let mut data1 = Vec::new();
    data1.extend_from_slice(&txid1);
    data1.extend_from_slice(&txid2);
    let hash1 = ferrous_node::primitives::hash::sha256d(&data1);

    let mut data2 = Vec::new();
    data2.extend_from_slice(&txid3);
    data2.extend_from_slice(&txid4);
    let hash2 = ferrous_node::primitives::hash::sha256d(&data2);

    let mut data3 = Vec::new();
    data3.extend_from_slice(&txid5);
    data3.extend_from_slice(&txid5);
    let hash3 = ferrous_node::primitives::hash::sha256d(&data3);

    // Level 2: sha256d(hash1 || hash2), sha256d(hash3 || hash3)
    let mut data4 = Vec::new();
    data4.extend_from_slice(&hash1);
    data4.extend_from_slice(&hash2);
    let hash4 = ferrous_node::primitives::hash::sha256d(&data4);

    let mut data5 = Vec::new();
    data5.extend_from_slice(&hash3);
    data5.extend_from_slice(&hash3);
    let hash5 = ferrous_node::primitives::hash::sha256d(&data5);

    // Level 3: sha256d(hash4 || hash5)
    let mut data6 = Vec::new();
    data6.extend_from_slice(&hash4);
    data6.extend_from_slice(&hash5);
    let expected = ferrous_node::primitives::hash::sha256d(&data6);

    assert_eq!(root, expected);
}

#[test]
fn test_witness_merkle_root() {
    let wtxid1 = [1u8; 32];
    let wtxid2 = [2u8; 32];
    let wtxids = vec![wtxid1, wtxid2];
    let root = compute_witness_merkle_root(&wtxids);

    // Should prepend zero hash and compute merkle root
    let mut expected_txids = vec![[0u8; 32]];
    expected_txids.extend_from_slice(&wtxids);
    let expected = compute_merkle_root(&expected_txids);

    assert_eq!(root, expected);
}

#[test]
fn test_witness_merkle_root_empty() {
    let wtxids: Vec<Hash256> = vec![];
    let root = compute_witness_merkle_root(&wtxids);

    // Should prepend zero hash and compute merkle root
    let expected_txids = vec![[0u8; 32]];
    let expected = compute_merkle_root(&expected_txids);

    assert_eq!(root, expected);
}

#[test]
fn test_deterministic_output() {
    let txid1 = [1u8; 32];
    let txid2 = [2u8; 32];
    let txid3 = [3u8; 32];
    let txids = vec![txid1, txid2, txid3];

    let root1 = compute_merkle_root(&txids);
    let root2 = compute_merkle_root(&txids);

    assert_eq!(root1, root2);
}

#[test]
fn test_order_matters() {
    let txid1 = [1u8; 32];
    let txid2 = [2u8; 32];
    let txid3 = [3u8; 32];

    let txids1 = vec![txid1, txid2, txid3];
    let txids2 = vec![txid3, txid2, txid1];

    let root1 = compute_merkle_root(&txids1);
    let root2 = compute_merkle_root(&txids2);

    assert_ne!(root1, root2);
}
