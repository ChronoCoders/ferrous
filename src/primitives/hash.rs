use sha2::{Digest, Sha256};

/// A 256-bit hash value.
pub type Hash256 = [u8; 32];

/// Computes the Bitcoin-style double SHA-256 hash of the provided data.
///
/// This function is defined as `SHA256(SHA256(data))` and is used for
/// transaction identifiers, block hashes, and Merkle tree nodes.
pub fn sha256d(data: &[u8]) -> Hash256 {
    let first = Sha256::digest(data);
    let second = Sha256::digest(first);

    let mut out = [0u8; 32];
    out.copy_from_slice(&second);
    out
}

/// Computes a tagged hash used for domain-separated hashing in signatures.
///
/// The construction follows `SHA256( SHA256(tag) || SHA256(tag) || msg )`
/// where `tag` is an arbitrary, application-specific string.
pub fn tagged_hash(tag: &str, msg: &[u8]) -> Hash256 {
    let tag_hash = Sha256::digest(tag.as_bytes());

    let mut hasher = Sha256::new();
    hasher.update(tag_hash);
    hasher.update(tag_hash);
    hasher.update(msg);

    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}
