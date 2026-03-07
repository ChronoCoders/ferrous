use ferrous_node::consensus::block::U256;
use ferrous_node::primitives::hash::Hash256;
use ferrous_node::storage::chain_state::ChainTip;
use ferrous_node::storage::{ChainStateStore, Database};
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_db() -> (TempDir, Arc<Database>) {
    let temp_dir = TempDir::new().unwrap();
    let db = Database::open(temp_dir.path()).unwrap();
    (temp_dir, Arc::new(db))
}

fn create_test_tip(height: u64) -> ChainTip {
    ChainTip {
        hash: [height as u8; 32],
        height,
        cumulative_work: U256::from(height * 1000),
    }
}

#[test]
fn test_chain_state_store_creation() {
    let (_temp, db) = create_test_db();
    let _store = ChainStateStore::new(db);
}

#[test]
fn test_empty_chain_state() {
    let (_temp, db) = create_test_db();
    let store = ChainStateStore::new(db);

    assert_eq!(store.get_tip().unwrap(), None);
    assert!(!store.is_initialized().unwrap());
}

#[test]
fn test_set_and_get_tip() {
    let (_temp, db) = create_test_db();
    let store = ChainStateStore::new(db);

    let tip = create_test_tip(100);
    store.set_tip(&tip).unwrap();

    let retrieved = store.get_tip().unwrap().unwrap();
    assert_eq!(retrieved, tip);
    assert!(store.is_initialized().unwrap());
}

#[test]
fn test_update_tip() {
    let (_temp, db) = create_test_db();
    let store = ChainStateStore::new(db);

    let tip1 = create_test_tip(100);
    store.set_tip(&tip1).unwrap();

    let tip2 = create_test_tip(101);
    store.set_tip(&tip2).unwrap();

    let retrieved = store.get_tip().unwrap().unwrap();
    assert_eq!(retrieved, tip2);
}

#[test]
fn test_tip_height_only() {
    let (_temp, db) = create_test_db();
    let store = ChainStateStore::new(db);

    let tip = create_test_tip(500);
    store.set_tip(&tip).unwrap();

    assert_eq!(store.get_tip_height().unwrap(), Some(500));
}

#[test]
fn test_tip_hash_only() {
    let (_temp, db) = create_test_db();
    let store = ChainStateStore::new(db);

    let tip = create_test_tip(100);
    store.set_tip(&tip).unwrap();

    let hash = store.get_tip_hash().unwrap().unwrap();
    assert_eq!(hash, tip.hash);
}

#[test]
fn test_best_header() {
    let (_temp, db) = create_test_db();
    let store = ChainStateStore::new(db);

    let hash: Hash256 = [42u8; 32];
    let height = 1000;

    store.set_best_header(&hash, height).unwrap();

    let (retrieved_hash, retrieved_height) = store.get_best_header().unwrap().unwrap();
    assert_eq!(retrieved_hash, hash);
    assert_eq!(retrieved_height, height);
}

#[test]
fn test_update_tip_with_reorg_verification() {
    let (_temp, db) = create_test_db();
    let store = ChainStateStore::new(db);

    let tip1 = create_test_tip(100);
    store.set_tip(&tip1).unwrap();

    let tip2 = create_test_tip(101);

    // Should succeed with correct old tip
    store.update_tip_with_reorg(&tip2, Some(&tip1)).unwrap();

    let retrieved = store.get_tip().unwrap().unwrap();
    assert_eq!(retrieved, tip2);
}

#[test]
fn test_update_tip_with_wrong_old_tip_fails() {
    let (_temp, db) = create_test_db();
    let store = ChainStateStore::new(db);

    let tip1 = create_test_tip(100);
    store.set_tip(&tip1).unwrap();

    let wrong_old = create_test_tip(99);
    let tip2 = create_test_tip(101);

    // Should fail with wrong old tip
    let result = store.update_tip_with_reorg(&tip2, Some(&wrong_old));
    assert!(result.is_err());
}

#[test]
fn test_clear_chain_state() {
    let (_temp, db) = create_test_db();
    let store = ChainStateStore::new(db);

    let tip = create_test_tip(100);
    store.set_tip(&tip).unwrap();

    let hash: Hash256 = [1u8; 32];
    store.set_best_header(&hash, 200).unwrap();

    assert!(store.is_initialized().unwrap());

    store.clear().unwrap();

    assert!(!store.is_initialized().unwrap());
    assert_eq!(store.get_tip().unwrap(), None);
    assert_eq!(store.get_best_header().unwrap(), None);
}
