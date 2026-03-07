use crate::consensus::block::{Block, BlockHeader};
use crate::consensus::utxo::{OutPoint, UtxoEntry};
use crate::primitives::hash::Hash256;
use crate::storage::{BlockStore, Database, CF_BLOCKS, CF_BLOCK_INDEX, CF_HEADERS, CF_UTXO};
use std::path::Path;
use std::sync::Arc;

pub struct BlockchainDB {
    db: Arc<Database>,
    blocks: BlockStore,
}

impl BlockchainDB {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let db = Arc::new(Database::open(path)?);
        let blocks = BlockStore::new(Arc::clone(&db));
        Ok(Self { db, blocks })
    }

    // ... tip methods ...

    // ... utxo methods ...

    pub fn put_header(&self, hash: &Hash256, header: &BlockHeader) -> Result<(), String> {
        use crate::primitives::serialize::Encode;
        let bytes = header.encode();
        self.db.put(CF_HEADERS, hash, &bytes)
    }

    pub fn get_header(&self, hash: &Hash256) -> Result<Option<BlockHeader>, String> {
        self.blocks.get_header(hash)
    }

    pub fn put_block(&self, hash: &Hash256, block: &Block) -> Result<(), String> {
        let bytes = bincode::serialize(block).map_err(|e| e.to_string())?;
        self.db.put(CF_BLOCKS, hash, &bytes)
    }

    pub fn get_block(&self, hash: &Hash256) -> Result<Option<Block>, String> {
        self.blocks.get_block(hash)
    }

    pub fn put_height_index(&self, height: u32, hash: &Hash256) -> Result<(), String> {
        let height_u64 = height as u64;
        self.db.put(CF_BLOCK_INDEX, &height_u64.to_le_bytes(), hash)
    }

    pub fn get_hash_at_height(&self, height: u32) -> Result<Option<Hash256>, String> {
        let height_u64 = height as u64;
        let bytes = self.db.get(CF_BLOCK_INDEX, &height_u64.to_le_bytes())?;
        match bytes {
            Some(b) => {
                let hash: Hash256 = b.try_into().map_err(|_| "Invalid hash bytes")?;
                Ok(Some(hash))
            }
            None => Ok(None),
        }
    }

    pub fn put_utxo(&self, outpoint: &OutPoint, entry: &UtxoEntry) -> Result<(), String> {
        let key = bincode::serialize(outpoint).map_err(|e| e.to_string())?;
        let value = bincode::serialize(entry).map_err(|e| e.to_string())?;
        self.db.put(CF_UTXO, &key, &value)
    }

    pub fn delete_utxo(&self, outpoint: &OutPoint) -> Result<(), String> {
        let key = bincode::serialize(outpoint).map_err(|e| e.to_string())?;
        self.db.delete(CF_UTXO, &key)
    }

    pub fn clear_utxos(&self) -> Result<(), String> {
        // RocksDB doesn't have clear_cf. Iterate and delete.
        // Or drop and recreate CF? No API for that here.
        // Using batch delete.
        let mut batch = self.db.batch();
        let iter = self.db.iter(CF_UTXO)?;
        for (key, _) in iter {
            batch.delete(CF_UTXO, &key)?;
        }
        batch.commit()
    }
}
