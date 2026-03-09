use crate::consensus::block::{Block, BlockHeader, U256};
use crate::primitives::hash::Hash256;
use crate::storage::{Database, CF_BLOCKS, CF_BLOCK_INDEX, CF_HEADERS};
use std::sync::Arc;

/// Block metadata for index (Internal use or if we want to store it separately)
#[derive(Debug, Clone)]
pub struct BlockMeta {
    pub height: u64,
    pub hash: Hash256,
    pub cumulative_work: U256,
}

impl BlockMeta {
    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.height.to_le_bytes());
        bytes.extend_from_slice(&self.hash);
        // U256 is wrapper around [u8; 32]
        bytes.extend_from_slice(&self.cumulative_work.0);
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 8 + 32 + 32 {
            return Err("Invalid BlockMeta bytes".to_string());
        }

        let height = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let hash = bytes[8..40].try_into().map_err(|_| "Invalid hash bytes")?;

        let mut work_bytes = [0u8; 32];
        work_bytes.copy_from_slice(&bytes[40..72]);
        let cumulative_work = U256(work_bytes);

        Ok(Self {
            height,
            hash,
            cumulative_work,
        })
    }
}

/// Block storage interface
pub struct BlockStore {
    db: Arc<Database>,
}

impl BlockStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Store block
    pub fn store_block(
        &self,
        block: &Block,
        height: u64,
        _cumulative_work: U256,
    ) -> Result<(), String> {
        let block_hash = block.header.hash();

        let block_bytes = bincode::serialize(block).map_err(|e| e.to_string())?;

        let mut batch = self.db.batch();
        batch.put(CF_BLOCKS, &block_hash, &block_bytes)?;

        use crate::primitives::serialize::Encode;
        let header_bytes = block.header.encode();
        batch.put(CF_HEADERS, &block_hash, &header_bytes)?;

        batch.put(CF_BLOCK_INDEX, &height.to_le_bytes(), &block_hash)?;

        batch.commit()
    }

    /// Get block by hash
    pub fn get_block(&self, hash: &Hash256) -> Result<Option<Block>, String> {
        let bytes = self.db.get(CF_BLOCKS, hash)?;

        match bytes {
            Some(b) => {
                let block = bincode::deserialize(&b).map_err(|e| e.to_string())?;
                Ok(Some(block))
            }
            None => Ok(None),
        }
    }

    /// Get block by height
    pub fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, String> {
        // Get hash from index
        let hash_bytes = self.db.get(CF_BLOCK_INDEX, &height.to_le_bytes())?;

        match hash_bytes {
            Some(h) => {
                let hash: Hash256 = h.try_into().map_err(|_| "Invalid hash")?;
                self.get_block(&hash)
            }
            None => Ok(None),
        }
    }

    /// Get header by hash
    pub fn get_header(&self, hash: &Hash256) -> Result<Option<BlockHeader>, String> {
        let bytes = self.db.get(CF_HEADERS, hash)?;

        match bytes {
            Some(b) => {
                use crate::primitives::serialize::Decode;
                let (header, _) = BlockHeader::decode(&b).map_err(|e| format!("{:?}", e))?;
                Ok(Some(header))
            }
            None => Ok(None),
        }
    }

    /// Check if block exists
    pub fn has_block(&self, hash: &Hash256) -> Result<bool, String> {
        Ok(self.db.get(CF_BLOCKS, hash)?.is_some())
    }

    /// Get blockchain height (highest stored block)
    pub fn get_height(&self) -> Result<Option<u64>, String> {
        let items = self.db.iter(CF_BLOCK_INDEX)?;

        if items.is_empty() {
            return Ok(None);
        }

        // Keys are u64 LE bytes
        let max_height = items
            .iter()
            .map(|(key, _)| {
                if key.len() >= 8 {
                    u64::from_le_bytes(key[..8].try_into().unwrap())
                } else {
                    0
                }
            })
            .max();

        Ok(max_height)
    }
}
