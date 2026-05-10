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

#[cfg(target_os = "linux")]
pub fn randomx_pow_hash(input: &[u8], epoch_key: &[u8]) -> [u8; 32] {
    use randomx_rs::{RandomXCache, RandomXFlag, RandomXVM};
    use std::cell::RefCell;

    thread_local! {
        static VM: RefCell<Option<(Vec<u8>, RandomXVM)>> = RefCell::new(None);
    }

    VM.with(|cell| {
        let mut slot = cell.borrow_mut();
        let needs_init = slot.as_ref().map_or(true, |(k, _)| k != epoch_key);
        if needs_init {
            let flags = RandomXFlag::get_recommended_flags();
            let cache = RandomXCache::new(flags, epoch_key).expect("RandomX cache init");
            let vm = RandomXVM::new(flags, Some(cache), None).expect("RandomX VM init");
            *slot = Some((epoch_key.to_vec(), vm));
        }
        let (_, vm) = slot.as_mut().unwrap();
        let result = vm.calculate_hash(input).expect("RandomX hash");
        let mut out = [0u8; 32];
        out.copy_from_slice(&result[..32]);
        out
    })
}

#[cfg(not(target_os = "linux"))]
pub fn randomx_pow_hash(_input: &[u8], _epoch_key: &[u8]) -> [u8; 32] {
    panic!("RandomX PoW is only supported on Linux. Run on the VPS.")
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
