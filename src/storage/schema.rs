/// Column families for RocksDB
pub mod cf {
    pub const BLOCKS: &str = "blocks"; // hash -> BlockData
    pub const HEADERS: &str = "headers"; // hash -> BlockHeader
    pub const HEIGHT_INDEX: &str = "height"; // height(u32) -> hash
    pub const UTXOS: &str = "utxos"; // OutPoint -> UtxoEntry
    pub const METADATA: &str = "metadata"; // "tip", "height", etc.
}

/// Metadata keys
pub mod meta {
    pub const TIP_HASH: &[u8] = b"tip_hash";
    pub const TIP_HEIGHT: &[u8] = b"tip_height";
}

/// Serialize height to bytes (big-endian for proper sorting)
pub fn height_to_key(height: u32) -> [u8; 4] {
    height.to_be_bytes()
}

/// Deserialize height from bytes
pub fn key_to_height(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().unwrap())
}
