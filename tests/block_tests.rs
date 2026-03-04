use ferrous_node::consensus::block::{BlockHeader, TargetError, U256};
use ferrous_node::primitives::hash::{sha256d, Hash256};
use ferrous_node::primitives::serialize::{Decode, Encode};

fn sample_header(n_bits: u32, nonce: u64) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_block_hash: [1u8; 32],
        merkle_root: [2u8; 32],
        timestamp: 1_700_000_000,
        n_bits,
        nonce,
    }
}

fn header_manual_hash(header: &BlockHeader) -> Hash256 {
    let encoded = header.encode();
    sha256d(&encoded)
}

fn manual_target_from_nbits(n_bits: u32) -> Result<U256, TargetError> {
    let exponent = (n_bits >> 24) as u8;
    let mantissa = n_bits & 0x00FF_FFFF;

    if mantissa >= 0x0080_0000 {
        return Err(TargetError::NegativeEncoding);
    }

    if exponent > 32 {
        return Err(TargetError::ExponentTooLarge);
    }

    let mut bytes = [0u8; 32];

    if exponent > 3 {
        let shift_bytes = usize::from(exponent - 3);

        if shift_bytes + 3 > 32 {
            return Err(TargetError::Overflow);
        }

        let mant_bytes = [
            (mantissa & 0xFF) as u8,
            ((mantissa >> 8) & 0xFF) as u8,
            ((mantissa >> 16) & 0xFF) as u8,
        ];

        for (i, b) in mant_bytes.iter().enumerate() {
            bytes[shift_bytes + i] = *b;
        }
    } else {
        let shift = 8 * (3_u32.saturating_sub(u32::from(exponent)));
        let value = mantissa >> shift;

        let mant_bytes = [
            (value & 0xFF) as u8,
            ((value >> 8) & 0xFF) as u8,
            ((value >> 16) & 0xFF) as u8,
        ];

        for (i, b) in mant_bytes.iter().enumerate() {
            if i < 32 {
                bytes[i] = *b;
            }
        }
    }

    Ok(U256::from_le_bytes(bytes))
}

#[test]
fn test_block_header_encoded_size_is_88_bytes() {
    let header = sample_header(0x1d00ffff, 0);
    let encoded = header.encode();
    assert_eq!(encoded.len(), 88);
    assert_eq!(header.encoded_size(), 88);
}

#[test]
fn test_block_header_roundtrip() {
    let header = sample_header(0x1b0404cb, 42);
    let encoded = header.encode();
    let (decoded, consumed) = BlockHeader::decode(&encoded).expect("decode failed");
    assert_eq!(consumed, 88);
    assert_eq!(decoded, header);
}

#[test]
fn test_block_header_field_ordering() {
    let header = sample_header(0x1d00ffff, 99);
    let encoded = header.encode();

    let (version, c1) = u32::decode(&encoded).expect("version decode failed");
    let (prev_hash, c2) = <[u8; 32]>::decode(&encoded[c1..]).expect("prev hash decode failed");
    let (merkle_root, c3) = <[u8; 32]>::decode(&encoded[c1 + c2..]).expect("merkle decode failed");
    let (timestamp, c4) = u64::decode(&encoded[c1 + c2 + c3..]).expect("timestamp decode failed");
    let (n_bits, c5) = u32::decode(&encoded[c1 + c2 + c3 + c4..]).expect("n_bits decode failed");
    let (nonce, c6) = u64::decode(&encoded[c1 + c2 + c3 + c4 + c5..]).expect("nonce decode failed");

    let consumed = c1 + c2 + c3 + c4 + c5 + c6;

    assert_eq!(consumed, 88);
    assert_eq!(version, header.version);
    assert_eq!(prev_hash, header.prev_block_hash);
    assert_eq!(merkle_root, header.merkle_root);
    assert_eq!(timestamp, header.timestamp);
    assert_eq!(n_bits, header.n_bits);
    assert_eq!(nonce, header.nonce);
}

#[test]
fn test_block_header_decode_truncated() {
    let header = sample_header(0x1d00ffff, 0);
    let mut encoded = header.encode();
    encoded.pop();

    let result = BlockHeader::decode(&encoded);
    assert!(result.is_err());
}

#[test]
fn test_header_hash_matches_sha256d_of_encoded_header() {
    let header = sample_header(0x1d00ffff, 0);
    let expected = header_manual_hash(&header);
    let actual = header.hash();
    assert_eq!(actual, expected);
}

#[test]
fn test_header_hash_is_deterministic() {
    let header1 = sample_header(0x1d00ffff, 1);
    let header2 = sample_header(0x1d00ffff, 1);
    assert_eq!(header1.hash(), header2.hash());
}

#[test]
fn test_header_hash_changes_with_nonce() {
    let header1 = sample_header(0x1d00ffff, 1);
    let header2 = sample_header(0x1d00ffff, 2);
    assert_ne!(header1.hash(), header2.hash());
}

#[test]
fn test_target_decoding_difficulty_one() {
    let header = sample_header(0x1d00ffff, 0);
    let target = header.target().expect("target failed");

    let expected = manual_target_from_nbits(0x1d00ffff).expect("manual target failed");
    assert_eq!(target, expected);

    let bytes = target.0;
    assert_eq!(bytes[26], 0xFF);
    assert_eq!(bytes[27], 0xFF);
    assert_eq!(bytes[28], 0x00);
}

#[test]
fn test_target_decoding_example_value() {
    let header = sample_header(0x1b0404cb, 0);
    let target = header.target().expect("target failed");

    let expected = manual_target_from_nbits(0x1b0404cb).expect("manual target failed");
    assert_eq!(target, expected);
}

#[test]
fn test_target_negative_encoding_rejected() {
    let header = sample_header(0x1dffff00, 0);
    let err = header.target().unwrap_err();
    assert!(matches!(err, TargetError::NegativeEncoding));
}

#[test]
fn test_target_exponent_too_large() {
    let header = sample_header(0x2100ffff, 0);
    let err = header.target().unwrap_err();
    assert!(matches!(err, TargetError::ExponentTooLarge));
}

#[test]
fn test_u256_ordering() {
    let mut a_bytes = [0u8; 32];
    a_bytes[0] = 1;
    let a = U256::from_le_bytes(a_bytes);

    let mut b_bytes = [0u8; 32];
    b_bytes[31] = 1;
    let b = U256::from_le_bytes(b_bytes);

    assert!(a < b);
    assert!(b > a);
    assert_eq!(a, a);
}

#[test]
fn test_proof_of_work_comparison_logic() {
    let header = sample_header(0x1d00ffff, 0);
    let target = header.target().expect("target failed");
    let hash = header.hash();
    let hash_value = U256::from_le_bytes(hash);

    let expected = hash_value <= target;
    let actual = header.check_proof_of_work().expect("pow failed");
    assert_eq!(actual, expected);
}

#[test]
fn test_proof_of_work_for_multiple_headers() {
    let headers = [
        sample_header(0x1d00ffff, 0),
        sample_header(0x1d00ffff, 1),
        sample_header(0x1b0404cb, 0),
        sample_header(0x1b0404cb, 100),
    ];

    for header in &headers {
        let target = header.target().expect("target failed");
        let hash = header.hash();
        let hash_value = U256::from_le_bytes(hash);
        let expected = hash_value <= target;
        let actual = header.check_proof_of_work().expect("pow failed");
        assert_eq!(actual, expected);
    }
}
