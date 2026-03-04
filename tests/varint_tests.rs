use ferrous_node::primitives::varint::{decode, encode, VarIntError};

fn lcg(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    *seed
}

#[test]
fn test_encode_zero() {
    assert_eq!(encode(0), vec![0x00]);
}

#[test]
fn test_encode_one() {
    assert_eq!(encode(1), vec![0x01]);
}

#[test]
fn test_encode_252() {
    assert_eq!(encode(252), vec![0xFC]);
}

#[test]
fn test_encode_253_boundary() {
    assert_eq!(encode(253), vec![0xFD, 0xFD, 0x00]);
}

#[test]
fn test_encode_65535() {
    assert_eq!(encode(65535), vec![0xFD, 0xFF, 0xFF]);
}

#[test]
fn test_encode_65536_boundary() {
    let encoded = encode(65536);
    assert_eq!(encoded[0], 0xFE);
}

#[test]
fn test_encode_u32_max() {
    let encoded = encode(u32::MAX as u64);
    assert_eq!(encoded[0], 0xFE);
}

#[test]
fn test_encode_u32_plus_one() {
    let value = (u32::MAX as u64) + 1;
    let encoded = encode(value);
    assert_eq!(encoded[0], 0xFF);
}

#[test]
fn test_decode_zero() {
    let (value, consumed) = decode(&[0x00]).expect("decode failed");
    assert_eq!(value, 0);
    assert_eq!(consumed, 1);
}

#[test]
fn test_decode_252() {
    let (value, consumed) = decode(&[0xFC]).expect("decode failed");
    assert_eq!(value, 252);
    assert_eq!(consumed, 1);
}

#[test]
fn test_decode_253_boundary() {
    let (value, consumed) = decode(&[0xFD, 0xFD, 0x00]).expect("decode failed");
    assert_eq!(value, 253);
    assert_eq!(consumed, 3);
}

#[test]
fn test_decode_65535() {
    let (value, consumed) = decode(&[0xFD, 0xFF, 0xFF]).expect("decode failed");
    assert_eq!(value, 65535);
    assert_eq!(consumed, 3);
}

#[test]
fn test_decode_65536_boundary() {
    let bytes = [0xFE, 0x00, 0x00, 0x01, 0x00];
    let (value, consumed) = decode(&bytes).expect("decode failed");
    assert_eq!(value, 65536);
    assert_eq!(consumed, 5);
}

#[test]
fn test_decode_u32_max() {
    let value = u32::MAX;
    let mut bytes = vec![0xFE];
    bytes.extend_from_slice(&value.to_le_bytes());
    let (decoded, consumed) = decode(&bytes).expect("decode failed");
    assert_eq!(decoded, value as u64);
    assert_eq!(consumed, 5);
}

#[test]
fn test_decode_u64_max() {
    let value = u64::MAX;
    let mut bytes = vec![0xFF];
    bytes.extend_from_slice(&value.to_le_bytes());
    let (decoded, consumed) = decode(&bytes).expect("decode failed");
    assert_eq!(decoded, value);
    assert_eq!(consumed, 9);
}

#[test]
fn test_non_minimal_u16_encoding_for_one() {
    let bytes = [0xFD, 0x01, 0x00];
    let err = decode(&bytes).unwrap_err();
    assert!(matches!(err, VarIntError::NonMinimalEncoding));
}

#[test]
fn test_non_minimal_u32_encoding_for_small() {
    let bytes = [0xFE, 0xFF, 0xFF, 0x00, 0x00];
    let err = decode(&bytes).unwrap_err();
    assert!(matches!(err, VarIntError::NonMinimalEncoding));
}

#[test]
fn test_non_minimal_u64_encoding_for_small() {
    let value = u32::MAX;
    let mut bytes = vec![0xFF];
    bytes.extend_from_slice(&(value as u64).to_le_bytes());
    let err = decode(&bytes).unwrap_err();
    assert!(matches!(err, VarIntError::NonMinimalEncoding));
}

#[test]
fn test_truncated_fd_prefixes() {
    assert!(matches!(
        decode(&[0xFD]).unwrap_err(),
        VarIntError::UnexpectedEof
    ));
    assert!(matches!(
        decode(&[0xFD, 0x00]).unwrap_err(),
        VarIntError::UnexpectedEof
    ));
}

#[test]
fn test_truncated_fe_prefixes() {
    assert!(matches!(
        decode(&[0xFE]).unwrap_err(),
        VarIntError::UnexpectedEof
    ));
    assert!(matches!(
        decode(&[0xFE, 0x00, 0x00, 0x00]).unwrap_err(),
        VarIntError::UnexpectedEof
    ));
}

#[test]
fn test_truncated_ff_prefixes() {
    assert!(matches!(
        decode(&[0xFF]).unwrap_err(),
        VarIntError::UnexpectedEof
    ));
    assert!(matches!(
        decode(&[0xFF, 0, 0, 0, 0, 0, 0, 0]).unwrap_err(),
        VarIntError::UnexpectedEof
    ));
}

#[test]
fn test_roundtrip_random_values() {
    let mut seed = 1u64;
    for _ in 0..1000 {
        let value = lcg(&mut seed);
        let encoded = encode(value);
        let (decoded, consumed) = decode(&encoded).expect("decode failed");
        assert_eq!(decoded, value);
        assert_eq!(consumed, encoded.len());
    }
}
