use rocksdb::{IteratorMode, Options, WriteBatch, WriteOptions, DB};
use std::path::Path;
use std::sync::Arc;

/// Column family names
pub const CF_BLOCKS: &str = "blocks";
pub const CF_BLOCK_INDEX: &str = "block_index";
pub const CF_HEADERS: &str = "headers";
pub const CF_UTXO: &str = "utxo";
pub const CF_CHAIN_STATE: &str = "chain_state";
pub const CF_UNDO: &str = "undo";
pub const CF_BLOCK_META: &str = "block_meta";

pub type DbEntry = (Vec<u8>, Vec<u8>);

/// Database wrapper with column families
pub struct Database {
    db: Arc<DB>,
}

impl Database {
    /// Open or create database at path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        // Define column families
        let cfs = vec![
            CF_BLOCKS,
            CF_BLOCK_INDEX,
            CF_HEADERS,
            CF_BLOCK_META,
            CF_UTXO,
            CF_CHAIN_STATE,
            CF_UNDO,
        ];

        let db = DB::open_cf(&opts, path, &cfs)
            .map_err(|e| format!("Failed to open database: {}", e))?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Get value from column family
    pub fn get(&self, cf: &str, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let cf_handle = self
            .db
            .cf_handle(cf)
            .ok_or_else(|| format!("Column family {} not found", cf))?;

        self.db
            .get_cf(cf_handle, key)
            .map_err(|e| format!("Get failed: {}", e))
    }

    /// Put value in column family
    pub fn put(&self, cf: &str, key: &[u8], value: &[u8]) -> Result<(), String> {
        let cf_handle = self
            .db
            .cf_handle(cf)
            .ok_or_else(|| format!("Column family {} not found", cf))?;

        self.db
            .put_cf(cf_handle, key, value)
            .map_err(|e| format!("Put failed: {}", e))
    }

    /// Delete key from column family
    pub fn delete(&self, cf: &str, key: &[u8]) -> Result<(), String> {
        let cf_handle = self
            .db
            .cf_handle(cf)
            .ok_or_else(|| format!("Column family {} not found", cf))?;

        self.db
            .delete_cf(cf_handle, key)
            .map_err(|e| format!("Delete failed: {}", e))
    }

    /// Create write batch for atomic operations
    pub fn batch(&self) -> DatabaseBatch {
        DatabaseBatch {
            db: Arc::clone(&self.db),
            batch: WriteBatch::default(),
        }
    }

    /// Iterator over keys in column family
    pub fn iter(&self, cf: &str) -> Result<Vec<DbEntry>, String> {
        let cf_handle = self
            .db
            .cf_handle(cf)
            .ok_or_else(|| format!("Column family {} not found", cf))?;

        let iter = self.db.iterator_cf(cf_handle, IteratorMode::Start);
        let mut result = Vec::new();
        for item in iter {
            match item {
                Ok((key, value)) => result.push((key.to_vec(), value.to_vec())),
                Err(e) => {
                    log::error!("Database iterator error: {}", e);
                    return Err(format!("Database iterator error: {}", e));
                }
            }
        }
        Ok(result)
    }
}

/// Atomic write batch
pub struct DatabaseBatch {
    db: Arc<DB>,
    batch: WriteBatch,
}

impl DatabaseBatch {
    /// Put in batch
    pub fn put(&mut self, cf: &str, key: &[u8], value: &[u8]) -> Result<(), String> {
        let cf_handle = self
            .db
            .cf_handle(cf)
            .ok_or_else(|| format!("Column family {} not found", cf))?;

        self.batch.put_cf(cf_handle, key, value);
        Ok(())
    }

    /// Delete in batch
    pub fn delete(&mut self, cf: &str, key: &[u8]) -> Result<(), String> {
        let cf_handle = self
            .db
            .cf_handle(cf)
            .ok_or_else(|| format!("Column family {} not found", cf))?;

        self.batch.delete_cf(cf_handle, key);
        Ok(())
    }

    /// Commit batch atomically. WAL is kept enabled for crash recovery; the
    /// per-write fsync is skipped (`sync: false`) to avoid 30-60 s stalls
    /// caused by RocksDB compaction pressure on the VPS disks.  The OS page
    /// cache holds the write until the next graceful flush or WAL replay.
    pub fn commit(self) -> Result<(), String> {
        let mut opts = WriteOptions::new();
        opts.set_sync(false);
        self.db
            .write_opt(self.batch, &opts)
            .map_err(|e| format!("Batch commit failed: {}", e))
    }
}
