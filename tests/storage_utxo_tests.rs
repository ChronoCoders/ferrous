use ferrous_node::consensus::transaction::TxOutput;
use ferrous_node::consensus::utxo::{OutPoint, UtxoEntry};
use ferrous_node::storage::{Database, UtxoStore};
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_db() -> (TempDir, Arc<Database>) {
    let temp_dir = TempDir::new().unwrap();
    let db = Database::open(temp_dir.path()).unwrap();
    (temp_dir, Arc::new(db))
}

fn create_test_outpoint(vout: u32) -> OutPoint {
    OutPoint {
        txid: [vout as u8; 32],
        vout,
    }
}

fn create_test_entry(value: u64, height: u64) -> UtxoEntry {
    UtxoEntry {
        output: TxOutput {
            value,
            script_pubkey: vec![0x76, 0xa9, 0x88, 0xac],
        },
        coinbase: false,
        height,
    }
}

#[test]
fn test_utxo_store_creation() {
    let (_temp, db) = create_test_db();
    let _store = UtxoStore::new(db);
}

#[test]
fn test_put_and_get_utxo() {
    let (_temp, db) = create_test_db();
    let store = UtxoStore::new(db);

    let outpoint = create_test_outpoint(0);
    let entry = create_test_entry(50_000, 100);

    store.put_utxo(&outpoint, &entry).unwrap();
    let retrieved = store.get_utxo(&outpoint).unwrap();

    assert_eq!(retrieved, Some(entry));
}

#[test]
fn test_delete_utxo() {
    let (_temp, db) = create_test_db();
    let store = UtxoStore::new(db);

    let outpoint = create_test_outpoint(0);
    let entry = create_test_entry(50_000, 100);

    store.put_utxo(&outpoint, &entry).unwrap();
    assert!(store.has_utxo(&outpoint).unwrap());

    store.delete_utxo(&outpoint).unwrap();
    assert!(!store.has_utxo(&outpoint).unwrap());
}

#[test]
fn test_has_utxo() {
    let (_temp, db) = create_test_db();
    let store = UtxoStore::new(db);

    let outpoint = create_test_outpoint(0);
    assert!(!store.has_utxo(&outpoint).unwrap());

    let entry = create_test_entry(50_000, 100);
    store.put_utxo(&outpoint, &entry).unwrap();
    assert!(store.has_utxo(&outpoint).unwrap());
}

#[test]
fn test_apply_block() {
    let (_temp, db) = create_test_db();
    let store = UtxoStore::new(db);

    // Create UTXOs
    let created = vec![
        (create_test_outpoint(0), create_test_entry(50_000, 100)),
        (create_test_outpoint(1), create_test_entry(25_000, 100)),
    ];

    // Spend existing UTXO (setup)
    let spent_outpoint = create_test_outpoint(99);
    store
        .put_utxo(&spent_outpoint, &create_test_entry(100_000, 99))
        .unwrap();

    let spent = vec![spent_outpoint];

    // Apply block atomically
    store.apply_block(&created, &spent).unwrap();

    // Verify created UTXOs exist
    assert!(store.has_utxo(&created[0].0).unwrap());
    assert!(store.has_utxo(&created[1].0).unwrap());

    // Verify spent UTXO removed
    assert!(!store.has_utxo(&spent[0]).unwrap());
}

#[test]
fn test_revert_block() {
    let (_temp, db) = create_test_db();
    let store = UtxoStore::new(db);

    // Setup: Create UTXOs from a block
    let created_outpoints = vec![create_test_outpoint(0), create_test_outpoint(1)];

    for outpoint in &created_outpoints {
        store
            .put_utxo(outpoint, &create_test_entry(50_000, 100))
            .unwrap();
    }

    // Restore spent UTXOs
    let restored = vec![(create_test_outpoint(99), create_test_entry(100_000, 99))];

    // Revert block atomically
    store.revert_block(&created_outpoints, &restored).unwrap();

    // Verify created UTXOs removed
    assert!(!store.has_utxo(&created_outpoints[0]).unwrap());
    assert!(!store.has_utxo(&created_outpoints[1]).unwrap());

    // Verify restored UTXOs exist
    assert!(store.has_utxo(&restored[0].0).unwrap());
}

#[test]
fn test_utxo_count() {
    let (_temp, db) = create_test_db();
    let store = UtxoStore::new(db);

    assert_eq!(store.get_utxo_count().unwrap(), 0);

    store
        .put_utxo(&create_test_outpoint(0), &create_test_entry(50_000, 100))
        .unwrap();
    assert_eq!(store.get_utxo_count().unwrap(), 1);

    store
        .put_utxo(&create_test_outpoint(1), &create_test_entry(25_000, 100))
        .unwrap();
    assert_eq!(store.get_utxo_count().unwrap(), 2);

    store.delete_utxo(&create_test_outpoint(0)).unwrap();
    assert_eq!(store.get_utxo_count().unwrap(), 1);
}

#[test]
fn test_missing_utxo() {
    let (_temp, db) = create_test_db();
    let store = UtxoStore::new(db);

    let outpoint = create_test_outpoint(0);
    let result = store.get_utxo(&outpoint).unwrap();

    assert_eq!(result, None);
}

#[test]
fn test_multiple_utxos() {
    let (_temp, db) = create_test_db();
    let store = UtxoStore::new(db);

    // Store 100 UTXOs
    for i in 0..100 {
        let outpoint = create_test_outpoint(i);
        let entry = create_test_entry((i as u64) * 1000, i as u64);
        store.put_utxo(&outpoint, &entry).unwrap();
    }

    assert_eq!(store.get_utxo_count().unwrap(), 100);

    // Verify random access
    let outpoint = create_test_outpoint(42);
    let entry = store.get_utxo(&outpoint).unwrap().unwrap();
    assert_eq!(entry.output.value, 42_000);
}

#[test]
fn test_coinbase_flag_persists() {
    let (_temp, db) = create_test_db();
    let store = UtxoStore::new(db);

    let outpoint = create_test_outpoint(0);
    let mut entry = create_test_entry(50_000, 100);
    entry.coinbase = true;

    store.put_utxo(&outpoint, &entry).unwrap();
    let retrieved = store.get_utxo(&outpoint).unwrap().unwrap();

    assert!(retrieved.coinbase);
}
