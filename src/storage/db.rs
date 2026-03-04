use crate::consensus::block::{BlockData, BlockHeader};
use crate::consensus::utxo::{OutPoint, UtxoEntry, UtxoSet};
use crate::primitives::hash::Hash256;
use crate::storage::schema::{cf, height_to_key, meta};
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use std::path::Path;
use std::sync::Arc;

pub struct BlockchainDB {
    db: Arc<DB>,
}

impl BlockchainDB {
    /// Open or create database at given path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cfs = vec![
            ColumnFamilyDescriptor::new(cf::BLOCKS, Options::default()),
            ColumnFamilyDescriptor::new(cf::HEADERS, Options::default()),
            ColumnFamilyDescriptor::new(cf::HEIGHT_INDEX, Options::default()),
            ColumnFamilyDescriptor::new(cf::UTXOS, Options::default()),
            ColumnFamilyDescriptor::new(cf::METADATA, Options::default()),
        ];

        let db = DB::open_cf_descriptors(&opts, path, cfs)
            .map_err(|e| format!("Failed to open database: {}", e))?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Store block header
    pub fn put_header(&self, hash: &Hash256, header: &BlockHeader) -> Result<(), String> {
        let cf = self
            .db
            .cf_handle(cf::HEADERS)
            .ok_or("Headers CF not found")?;

        let data = bincode::serialize(header).map_err(|e| format!("Serialization error: {}", e))?;

        self.db
            .put_cf(cf, hash, data)
            .map_err(|e| format!("DB write error: {}", e))
    }

    /// Get block header by hash
    pub fn get_header(&self, hash: &Hash256) -> Result<Option<BlockHeader>, String> {
        let cf = self
            .db
            .cf_handle(cf::HEADERS)
            .ok_or("Headers CF not found")?;

        match self
            .db
            .get_cf(cf, hash)
            .map_err(|e| format!("DB read error: {}", e))?
        {
            Some(data) => {
                let header = bincode::deserialize(&data)
                    .map_err(|e| format!("Deserialization error: {}", e))?;
                Ok(Some(header))
            }
            None => Ok(None),
        }
    }

    /// Store block data
    pub fn put_block(&self, hash: &Hash256, block: &BlockData) -> Result<(), String> {
        let cf = self.db.cf_handle(cf::BLOCKS).ok_or("Blocks CF not found")?;

        let data = bincode::serialize(block).map_err(|e| format!("Serialization error: {}", e))?;

        self.db
            .put_cf(cf, hash, data)
            .map_err(|e| format!("DB write error: {}", e))
    }

    /// Get block data by hash
    pub fn get_block(&self, hash: &Hash256) -> Result<Option<BlockData>, String> {
        let cf = self.db.cf_handle(cf::BLOCKS).ok_or("Blocks CF not found")?;

        match self
            .db
            .get_cf(cf, hash)
            .map_err(|e| format!("DB read error: {}", e))?
        {
            Some(data) => {
                let block = bincode::deserialize(&data)
                    .map_err(|e| format!("Deserialization error: {}", e))?;
                Ok(Some(block))
            }
            None => Ok(None),
        }
    }

    /// Store height -> hash mapping
    pub fn put_height_index(&self, height: u32, hash: &Hash256) -> Result<(), String> {
        let cf = self
            .db
            .cf_handle(cf::HEIGHT_INDEX)
            .ok_or("Height index CF not found")?;

        self.db
            .put_cf(cf, height_to_key(height), hash)
            .map_err(|e| format!("DB write error: {}", e))
    }

    /// Get hash at height
    pub fn get_hash_at_height(&self, height: u32) -> Result<Option<Hash256>, String> {
        let cf = self
            .db
            .cf_handle(cf::HEIGHT_INDEX)
            .ok_or("Height index CF not found")?;

        match self
            .db
            .get_cf(cf, height_to_key(height))
            .map_err(|e| format!("DB read error: {}", e))?
        {
            Some(data) => {
                let hash: Hash256 = data.try_into().map_err(|_| "Invalid hash length")?;
                Ok(Some(hash))
            }
            None => Ok(None),
        }
    }

    /// Store UTXO
    pub fn put_utxo(&self, outpoint: &OutPoint, entry: &UtxoEntry) -> Result<(), String> {
        let cf = self.db.cf_handle(cf::UTXOS).ok_or("UTXOs CF not found")?;

        let key =
            bincode::serialize(outpoint).map_err(|e| format!("Serialization error: {}", e))?;
        let value = bincode::serialize(entry).map_err(|e| format!("Serialization error: {}", e))?;

        self.db
            .put_cf(cf, key, value)
            .map_err(|e| format!("DB write error: {}", e))
    }

    /// Delete UTXO
    pub fn delete_utxo(&self, outpoint: &OutPoint) -> Result<(), String> {
        let cf = self.db.cf_handle(cf::UTXOS).ok_or("UTXOs CF not found")?;

        let key =
            bincode::serialize(outpoint).map_err(|e| format!("Serialization error: {}", e))?;

        self.db
            .delete_cf(cf, key)
            .map_err(|e| format!("DB write error: {}", e))
    }

    /// Get UTXO
    pub fn get_utxo(&self, outpoint: &OutPoint) -> Result<Option<UtxoEntry>, String> {
        let cf = self.db.cf_handle(cf::UTXOS).ok_or("UTXOs CF not found")?;

        let key =
            bincode::serialize(outpoint).map_err(|e| format!("Serialization error: {}", e))?;

        match self
            .db
            .get_cf(cf, key)
            .map_err(|e| format!("DB read error: {}", e))?
        {
            Some(data) => {
                let entry = bincode::deserialize(&data)
                    .map_err(|e| format!("Deserialization error: {}", e))?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    /// Clear all UTXOs
    pub fn clear_utxos(&self) -> Result<(), String> {
        let cf = self.db.cf_handle(cf::UTXOS).ok_or("UTXOs CF not found")?;
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);

        // We collect keys first to avoid iterator invalidation issues if any
        let keys: Vec<_> = iter.map(|item| item.unwrap().0).collect();

        let mut batch = rocksdb::WriteBatch::default();
        for key in keys {
            batch.delete_cf(cf, key);
        }

        self.db
            .write(batch)
            .map_err(|e| format!("DB write error: {}", e))
    }

    /// Load all UTXOs (for startup)
    pub fn load_utxo_set(&self) -> Result<UtxoSet, String> {
        let cf = self.db.cf_handle(cf::UTXOS).ok_or("UTXOs CF not found")?;

        let mut utxo_set = UtxoSet::new();
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, value) = item.map_err(|e| format!("Iterator error: {}", e))?;
            let outpoint: OutPoint =
                bincode::deserialize(&key).map_err(|e| format!("Deserialization error: {}", e))?;
            let entry: UtxoEntry = bincode::deserialize(&value)
                .map_err(|e| format!("Deserialization error: {}", e))?;
            utxo_set.insert(outpoint, entry);
        }

        Ok(utxo_set)
    }

    /// Load raw UTXO entries as (OutPoint, UtxoEntry) pairs
    pub fn load_utxo_set_raw(&self) -> Result<Vec<(OutPoint, UtxoEntry)>, String> {
        let cf = self.db.cf_handle(cf::UTXOS).ok_or("UTXOs CF not found")?;

        let mut entries = Vec::new();
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, value) = item.map_err(|e| format!("Iterator error: {}", e))?;
            let outpoint: OutPoint =
                bincode::deserialize(&key).map_err(|e| format!("Deserialization error: {}", e))?;
            let entry: UtxoEntry = bincode::deserialize(&value)
                .map_err(|e| format!("Deserialization error: {}", e))?;
            entries.push((outpoint, entry));
        }

        Ok(entries)
    }

    /// Set chain tip
    pub fn set_tip(&self, hash: &Hash256, height: u32) -> Result<(), String> {
        let cf = self
            .db
            .cf_handle(cf::METADATA)
            .ok_or("Metadata CF not found")?;

        self.db
            .put_cf(cf, meta::TIP_HASH, hash)
            .map_err(|e| format!("DB write error: {}", e))?;

        self.db
            .put_cf(cf, meta::TIP_HEIGHT, height.to_le_bytes())
            .map_err(|e| format!("DB write error: {}", e))?;

        Ok(())
    }

    /// Get chain tip
    pub fn get_tip(&self) -> Result<Option<(Hash256, u32)>, String> {
        let cf = self
            .db
            .cf_handle(cf::METADATA)
            .ok_or("Metadata CF not found")?;

        let hash = match self
            .db
            .get_cf(cf, meta::TIP_HASH)
            .map_err(|e| format!("DB read error: {}", e))?
        {
            Some(data) => {
                let hash: Hash256 = data.try_into().map_err(|_| "Invalid hash length")?;
                hash
            }
            None => return Ok(None),
        };

        let height = match self
            .db
            .get_cf(cf, meta::TIP_HEIGHT)
            .map_err(|e| format!("DB read error: {}", e))?
        {
            Some(data) => u32::from_le_bytes(data.try_into().map_err(|_| "Invalid height length")?),
            None => return Ok(None),
        };

        Ok(Some((hash, height)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_db_open_create() {
        let dir = TempDir::new().unwrap();
        let _db = BlockchainDB::open(dir.path()).unwrap();
        // Database created successfully
    }

    #[test]
    fn test_store_retrieve_header() {
        let dir = TempDir::new().unwrap();
        let db = BlockchainDB::open(dir.path()).unwrap();

        let header = BlockHeader {
            version: 1,
            prev_block_hash: [0u8; 32],
            merkle_root: [0u8; 32],
            timestamp: 1234567890,
            n_bits: 0x207fffff,
            nonce: 42,
        };

        let hash = header.hash();
        db.put_header(&hash, &header).unwrap();

        let retrieved = db.get_header(&hash).unwrap().unwrap();
        assert_eq!(retrieved.nonce, 42);
    }

    #[test]
    fn test_persistence_across_restart() {
        let dir = TempDir::new().unwrap();
        let hash = [1u8; 32];

        {
            let db = BlockchainDB::open(dir.path()).unwrap();
            db.set_tip(&hash, 100).unwrap();
        }

        // Reopen database
        {
            let db = BlockchainDB::open(dir.path()).unwrap();
            let (tip_hash, height) = db.get_tip().unwrap().unwrap();
            assert_eq!(tip_hash, hash);
            assert_eq!(height, 100);
        }
    }
}
