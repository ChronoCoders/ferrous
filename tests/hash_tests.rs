use ferrous_node::primitives::hash::{sha256d, tagged_hash};
use sha2::{Digest, Sha256};

fn manual_sha256d(data: &[u8]) -> [u8; 32] {
    let first = Sha256::digest(data);
    let second = Sha256::digest(first);
    let mut out = [0u8; 32];
    out.copy_from_slice(&second);
    out
}

fn manual_tagged_hash(tag: &str, msg: &[u8]) -> [u8; 32] {
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

#[test]
fn test_sha256d_empty() {
    let expected = manual_sha256d(b"");
    let actual = sha256d(b"");
    assert_eq!(actual, expected);
}

#[test]
fn test_sha256d_abc() {
    let expected = manual_sha256d(b"abc");
    let actual = sha256d(b"abc");
    assert_eq!(actual, expected);
}

#[test]
fn test_sha256d_quick_brown_fox() {
    let expected = manual_sha256d(b"The quick brown fox jumps over the lazy dog");
    let actual = sha256d(b"The quick brown fox jumps over the lazy dog");
    assert_eq!(actual, expected);
}

#[test]
fn test_sha256d_large_input_1mb() {
    let data = vec![0u8; 1024 * 1024];
    let expected = manual_sha256d(&data);
    let actual = sha256d(&data);
    assert_eq!(actual, expected);
}

#[test]
fn test_tagged_hash_basic() {
    let tag = "TestTag";
    let msg = b"hello world";
    let expected = manual_tagged_hash(tag, msg);
    let actual = tagged_hash(tag, msg);
    assert_eq!(actual, expected);
}

#[test]
fn test_tagged_hash_empty_message() {
    let tag = "TestTag";
    let msg: &[u8] = &[];
    let expected = manual_tagged_hash(tag, msg);
    let actual = tagged_hash(tag, msg);
    assert_eq!(actual, expected);
}

#[test]
fn test_tagged_hash_different_tags_differ() {
    let msg = b"same message";
    let a = tagged_hash("TagA", msg);
    let b = tagged_hash("TagB", msg);
    assert_ne!(a, b);
}

#[test]
fn test_tagged_hash_different_messages_differ() {
    let tag = "TestTag";
    let a = tagged_hash(tag, b"message one");
    let b = tagged_hash(tag, b"message two");
    assert_ne!(a, b);
}
