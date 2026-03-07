use crate::consensus::block::U256;
use crate::primitives::hash::Hash256;
use crate::storage::{Database, CF_CHAIN_STATE};
use std::sync::Arc;

// Chain state keys
const KEY_TIP_HASH: &[u8] = b"tip_hash";
const KEY_TIP_HEIGHT: &[u8] = b"tip_height";
const KEY_CUMULATIVE_WORK: &[u8] = b"cumulative_work";
const KEY_BEST_HEADER_HASH: &[u8] = b"best_header_hash";
const KEY_BEST_HEADER_HEIGHT: &[u8] = b"best_header_height";

/// Chain state metadata
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainTip {
    pub hash: Hash256,
    pub height: u64,
    pub cumulative_work: U256,
}

/// Chain state storage interface
pub struct ChainStateStore {
    db: Arc<Database>,
}

impl ChainStateStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Get current chain tip
    pub fn get_tip(&self) -> Result<Option<ChainTip>, String> {
        let hash = match self.db.get(CF_CHAIN_STATE, KEY_TIP_HASH)? {
            Some(h) => {
                let mut arr = [0u8; 32];
                if h.len() != 32 {
                    return Err("Invalid tip hash length".to_string());
                }
                arr.copy_from_slice(&h);
                arr
            }
            None => return Ok(None),
        };

        let height = match self.db.get(CF_CHAIN_STATE, KEY_TIP_HEIGHT)? {
            Some(h) => u64::from_le_bytes(h.try_into().map_err(|_| "Invalid height")?),
            None => return Ok(None),
        };

        let cumulative_work = match self.db.get(CF_CHAIN_STATE, KEY_CUMULATIVE_WORK)? {
            Some(w) => U256::from_bytes_le(&w),
            None => return Ok(None),
        };

        Ok(Some(ChainTip {
            hash,
            height,
            cumulative_work,
        }))
    }

    /// Set chain tip (atomic)
    pub fn set_tip(&self, tip: &ChainTip) -> Result<(), String> {
        let mut batch = self.db.batch();

        batch.put(CF_CHAIN_STATE, KEY_TIP_HASH, &tip.hash)?;
        batch.put(CF_CHAIN_STATE, KEY_TIP_HEIGHT, &tip.height.to_le_bytes())?;
        batch.put(
            CF_CHAIN_STATE,
            KEY_CUMULATIVE_WORK,
            &tip.cumulative_work.to_bytes_le(),
        )?;

        batch.commit()
    }

    /// Get best header (for headers-first sync)
    pub fn get_best_header(&self) -> Result<Option<(Hash256, u64)>, String> {
        let hash = match self.db.get(CF_CHAIN_STATE, KEY_BEST_HEADER_HASH)? {
            Some(h) => {
                let mut arr = [0u8; 32];
                if h.len() != 32 {
                    return Err("Invalid header hash length".to_string());
                }
                arr.copy_from_slice(&h);
                arr
            }
            None => return Ok(None),
        };

        let height = match self.db.get(CF_CHAIN_STATE, KEY_BEST_HEADER_HEIGHT)? {
            Some(h) => u64::from_le_bytes(h.try_into().map_err(|_| "Invalid height")?),
            None => return Ok(None),
        };

        Ok(Some((hash, height)))
    }

    /// Set best header (atomic)
    pub fn set_best_header(&self, hash: &Hash256, height: u64) -> Result<(), String> {
        let mut batch = self.db.batch();

        batch.put(CF_CHAIN_STATE, KEY_BEST_HEADER_HASH, hash)?;
        batch.put(
            CF_CHAIN_STATE,
            KEY_BEST_HEADER_HEIGHT,
            &height.to_le_bytes(),
        )?;

        batch.commit()
    }

    /// Update tip with reorg information (atomic)
    pub fn update_tip_with_reorg(
        &self,
        new_tip: &ChainTip,
        old_tip: Option<&ChainTip>,
    ) -> Result<(), String> {
        // Verify old tip matches if provided (safety check)
        if let Some(old) = old_tip {
            let current = self.get_tip()?;
            if current.as_ref() != Some(old) {
                return Err("Tip mismatch: expected old tip doesn't match current".to_string());
            }
        }

        self.set_tip(new_tip)
    }

    /// Clear all chain state (dangerous, for testing/reset only)
    pub fn clear(&self) -> Result<(), String> {
        let mut batch = self.db.batch();

        batch.delete(CF_CHAIN_STATE, KEY_TIP_HASH)?;
        batch.delete(CF_CHAIN_STATE, KEY_TIP_HEIGHT)?;
        batch.delete(CF_CHAIN_STATE, KEY_CUMULATIVE_WORK)?;
        batch.delete(CF_CHAIN_STATE, KEY_BEST_HEADER_HASH)?;
        batch.delete(CF_CHAIN_STATE, KEY_BEST_HEADER_HEIGHT)?;

        batch.commit()
    }

    /// Check if chain is initialized (has tip)
    pub fn is_initialized(&self) -> Result<bool, String> {
        Ok(self.get_tip()?.is_some())
    }

    /// Get tip height only (optimized)
    pub fn get_tip_height(&self) -> Result<Option<u64>, String> {
        match self.db.get(CF_CHAIN_STATE, KEY_TIP_HEIGHT)? {
            Some(h) => Ok(Some(u64::from_le_bytes(
                h.try_into().map_err(|_| "Invalid height")?,
            ))),
            None => Ok(None),
        }
    }

    /// Get tip hash only (optimized)
    pub fn get_tip_hash(&self) -> Result<Option<Hash256>, String> {
        match self.db.get(CF_CHAIN_STATE, KEY_TIP_HASH)? {
            Some(h) => {
                let mut arr = [0u8; 32];
                if h.len() != 32 {
                    return Err("Invalid hash length".to_string());
                }
                arr.copy_from_slice(&h);
                Ok(Some(arr))
            }
            None => Ok(None),
        }
    }
}
