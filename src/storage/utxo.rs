use crate::consensus::utxo::{OutPoint, UtxoEntry};
use crate::primitives::hash::Hash256;
use crate::storage::{Database, CF_UNDO, CF_UTXO};
use std::sync::Arc;

/// UTXO set storage interface
pub struct UtxoStore {
    db: Arc<Database>,
}

impl UtxoStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Get UTXO by outpoint
    pub fn get_utxo(&self, outpoint: &OutPoint) -> Result<Option<UtxoEntry>, String> {
        let key = Self::outpoint_key(outpoint);
        let bytes = self.db.get(CF_UTXO, &key)?;

        match bytes {
            Some(b) => {
                let entry = Self::deserialize_entry(&b)?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    /// Store UTXO
    pub fn put_utxo(&self, outpoint: &OutPoint, entry: &UtxoEntry) -> Result<(), String> {
        let key = Self::outpoint_key(outpoint);
        let value = Self::serialize_entry(entry)?;
        self.db.put(CF_UTXO, &key, &value)
    }

    /// Delete UTXO
    pub fn delete_utxo(&self, outpoint: &OutPoint) -> Result<(), String> {
        let key = Self::outpoint_key(outpoint);
        self.db.delete(CF_UTXO, &key)
    }

    /// Check if UTXO exists
    pub fn has_utxo(&self, outpoint: &OutPoint) -> Result<bool, String> {
        let key = Self::outpoint_key(outpoint);
        Ok(self.db.get(CF_UTXO, &key)?.is_some())
    }

    /// Apply block to UTXO set (atomic)
    pub fn apply_block(
        &self,
        created: &[(OutPoint, UtxoEntry)],
        spent: &[OutPoint],
    ) -> Result<(), String> {
        let mut batch = self.db.batch();

        // Add new UTXOs
        for (outpoint, entry) in created {
            let key = Self::outpoint_key(outpoint);
            let value = Self::serialize_entry(entry)?;
            batch.put(CF_UTXO, &key, &value)?;
        }

        // Remove spent UTXOs
        for outpoint in spent {
            let key = Self::outpoint_key(outpoint);
            batch.delete(CF_UTXO, &key)?;
        }

        batch.commit()
    }

    /// Revert block from UTXO set (atomic)
    pub fn revert_block(
        &self,
        created: &[OutPoint],
        restored: &[(OutPoint, UtxoEntry)],
    ) -> Result<(), String> {
        let mut batch = self.db.batch();

        // Remove UTXOs created by this block
        for outpoint in created {
            let key = Self::outpoint_key(outpoint);
            batch.delete(CF_UTXO, &key)?;
        }

        // Restore UTXOs spent by this block
        for (outpoint, entry) in restored {
            let key = Self::outpoint_key(outpoint);
            let value = Self::serialize_entry(entry)?;
            batch.put(CF_UTXO, &key, &value)?;
        }

        batch.commit()
    }

    pub fn store_undo_data(
        &self,
        block_hash: &Hash256,
        spent_entries: &[(OutPoint, UtxoEntry)],
    ) -> Result<(), String> {
        let value = bincode::serialize(spent_entries)
            .map_err(|e| format!("Failed to serialize undo data: {}", e))?;
        self.db.put(CF_UNDO, block_hash, &value)
    }

    pub fn get_undo_data(
        &self,
        block_hash: &Hash256,
    ) -> Result<Option<Vec<(OutPoint, UtxoEntry)>>, String> {
        match self.db.get(CF_UNDO, block_hash)? {
            Some(bytes) => {
                let entries: Vec<(OutPoint, UtxoEntry)> = bincode::deserialize(&bytes)
                    .map_err(|e| format!("Failed to deserialize undo data: {}", e))?;
                Ok(Some(entries))
            }
            None => Ok(None),
        }
    }

    pub fn delete_undo_data(&self, block_hash: &Hash256) -> Result<(), String> {
        self.db.delete(CF_UNDO, block_hash)
    }

    /// Get UTXO set size
    pub fn get_utxo_count(&self) -> Result<usize, String> {
        let items = self.db.iter(CF_UTXO)?;
        Ok(items.len())
    }

    /// Export all UTXOs (for testing/debugging)
    pub fn export_utxos(&self) -> Result<Vec<(OutPoint, UtxoEntry)>, String> {
        let mut result = Vec::new();
        let iter = self.db.iter(CF_UTXO)?;

        for (key, value) in iter {
            if key.len() != 36 {
                continue; // Skip invalid keys
            }
            let mut txid = [0u8; 32];
            txid.copy_from_slice(&key[0..32]);
            let vout = u32::from_le_bytes(key[32..36].try_into().unwrap());
            let outpoint = OutPoint { txid, vout };

            let entry = Self::deserialize_entry(&value)?;
            result.push((outpoint, entry));
        }
        Ok(result)
    }

    /// Serialize outpoint to key
    fn outpoint_key(outpoint: &OutPoint) -> Vec<u8> {
        let mut key = Vec::with_capacity(36);
        key.extend_from_slice(&outpoint.txid);
        key.extend_from_slice(&outpoint.vout.to_le_bytes());
        key
    }

    /// Serialize UTXO entry
    fn serialize_entry(entry: &UtxoEntry) -> Result<Vec<u8>, String> {
        bincode::serialize(entry).map_err(|e| format!("Failed to serialize UtxoEntry: {}", e))
    }

    /// Deserialize UTXO entry
    fn deserialize_entry(bytes: &[u8]) -> Result<UtxoEntry, String> {
        bincode::deserialize(bytes).map_err(|e| format!("Failed to deserialize UtxoEntry: {}", e))
    }
}
