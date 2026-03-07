use ferrous_node::consensus::block::{Block, BlockHeader, U256};
use ferrous_node::storage::{BlockStore, Database, CF_BLOCKS};
use std::sync::Arc;
use tempfile::TempDir;

fn create_dummy_block(height: u64) -> Block {
    // Create dummy header
    let header = BlockHeader {
        version: 1,
        prev_block_hash: [0u8; 32],
        merkle_root: [0u8; 32],
        timestamp: 123456789,
        n_bits: 0x1d00ffff,
        nonce: height, // Use height as nonce to make unique hash
    };

    Block {
        header,
        transactions: vec![],
    }
}

#[test]
fn test_database_open() {
    let temp_dir = TempDir::new().unwrap();
    let db = Database::open(temp_dir.path());
    assert!(db.is_ok());
}

#[test]
fn test_block_storage() {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(Database::open(temp_dir.path()).unwrap());
    let store = BlockStore::new(db);

    let block = create_dummy_block(1);
    let work = U256([0u8; 32]);

    let res = store.store_block(&block, 1, work);
    assert!(res.is_ok());

    // Verify stored
    let hash = block.header.hash();
    assert!(store.has_block(&hash).unwrap());
}

#[test]
fn test_block_retrieval_by_hash() {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(Database::open(temp_dir.path()).unwrap());
    let store = BlockStore::new(db);

    let block = create_dummy_block(2);
    let work = U256([0u8; 32]);
    let hash = block.header.hash();

    store.store_block(&block, 2, work).unwrap();

    let retrieved = store.get_block(&hash).unwrap().unwrap();
    assert_eq!(retrieved.header.nonce, 2);
    assert_eq!(retrieved.header.hash(), hash);
}

#[test]
fn test_block_retrieval_by_height() {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(Database::open(temp_dir.path()).unwrap());
    let store = BlockStore::new(db);

    let block = create_dummy_block(3);
    let work = U256([0u8; 32]);

    store.store_block(&block, 3, work).unwrap();

    let retrieved = store.get_block_by_height(3).unwrap().unwrap();
    assert_eq!(retrieved.header.nonce, 3);
}

#[test]
fn test_header_storage_retrieval() {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(Database::open(temp_dir.path()).unwrap());
    let store = BlockStore::new(db);

    let block = create_dummy_block(4);
    let work = U256([0u8; 32]);
    let hash = block.header.hash();

    store.store_block(&block, 4, work).unwrap();

    let header = store.get_header(&hash).unwrap().unwrap();
    assert_eq!(header.nonce, 4);
}

#[test]
fn test_has_block() {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(Database::open(temp_dir.path()).unwrap());
    let store = BlockStore::new(db);

    let block = create_dummy_block(5);
    let work = U256([0u8; 32]);
    let hash = block.header.hash();

    assert!(!store.has_block(&hash).unwrap());

    store.store_block(&block, 5, work).unwrap();

    assert!(store.has_block(&hash).unwrap());
}

#[test]
fn test_get_height() {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(Database::open(temp_dir.path()).unwrap());
    let store = BlockStore::new(db);

    // Empty
    assert!(store.get_height().unwrap().is_none());

    // Single
    let block1 = create_dummy_block(1);
    store.store_block(&block1, 1, U256([0u8; 32])).unwrap();
    assert_eq!(store.get_height().unwrap(), Some(1));

    // Multiple
    let block2 = create_dummy_block(2);
    store.store_block(&block2, 2, U256([0u8; 32])).unwrap();
    assert_eq!(store.get_height().unwrap(), Some(2));

    // Gap (should return max height)
    let block5 = create_dummy_block(5);
    store.store_block(&block5, 5, U256([0u8; 32])).unwrap();
    assert_eq!(store.get_height().unwrap(), Some(5));
}

#[test]
fn test_block_overwrite_metadata() {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(Database::open(temp_dir.path()).unwrap());
    let store = BlockStore::new(db);

    let block = create_dummy_block(6);
    // let hash = block.header.hash();

    store.store_block(&block, 6, U256([0u8; 32])).unwrap();

    // Retrieve by height 6
    let retrieved = store.get_block_by_height(6).unwrap().unwrap();
    assert_eq!(retrieved.header.nonce, 6);

    // Overwrite: Store same block at height 7
    store.store_block(&block, 7, U256([0u8; 32])).unwrap();

    let retrieved7 = store.get_block_by_height(7).unwrap().unwrap();
    assert_eq!(retrieved7.header.nonce, 6);
}

#[test]
fn test_missing_block() {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(Database::open(temp_dir.path()).unwrap());
    let store = BlockStore::new(db);

    let hash = [0u8; 32];
    assert!(store.get_block(&hash).unwrap().is_none());
    assert!(store.get_header(&hash).unwrap().is_none());
}

#[test]
fn test_atomic_batch() {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(Database::open(temp_dir.path()).unwrap());
    let _store = BlockStore::new(db.clone());

    // Manual batch test
    let mut batch = db.batch();
    batch.put(CF_BLOCKS, &[1, 2, 3], &[4, 5, 6]).unwrap();
    // Not committed yet
    assert!(db.get(CF_BLOCKS, &[1, 2, 3]).unwrap().is_none());

    batch.commit().unwrap();
    assert_eq!(
        db.get(CF_BLOCKS, &[1, 2, 3]).unwrap().unwrap(),
        vec![4, 5, 6]
    );
}
