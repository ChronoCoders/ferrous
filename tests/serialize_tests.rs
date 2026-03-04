use ferrous_node::primitives::serialize::{
    decode_u16_le, decode_u32_le, decode_u64_le, encode_u16_le, encode_u32_le, encode_u64_le,
    Decode, DecodeError, Encode,
};

#[test]
fn test_encode_decode_u8() {
    let value: u8 = 0xAB;
    let encoded = value.encode();
    assert_eq!(encoded, vec![0xAB]);

    let (decoded, consumed) = u8::decode(&encoded).expect("decode failed");
    assert_eq!(decoded, value);
    assert_eq!(consumed, 1);
}

#[test]
fn test_encode_decode_u16_le() {
    let value: u16 = 0x1234;
    let bytes = encode_u16_le(value);
    assert_eq!(bytes, [0x34, 0x12]);

    let decoded = decode_u16_le(&bytes).expect("decode failed");
    assert_eq!(decoded, value);

    let (decoded2, consumed) = u16::decode(&bytes).expect("trait decode failed");
    assert_eq!(decoded2, value);
    assert_eq!(consumed, 2);
}

#[test]
fn test_encode_decode_u32_le() {
    let value: u32 = 0x12345678;
    let bytes = encode_u32_le(value);
    assert_eq!(bytes, [0x78, 0x56, 0x34, 0x12]);

    let decoded = decode_u32_le(&bytes).expect("decode failed");
    assert_eq!(decoded, value);

    let (decoded2, consumed) = u32::decode(&bytes).expect("trait decode failed");
    assert_eq!(decoded2, value);
    assert_eq!(consumed, 4);
}

#[test]
fn test_encode_decode_u64_le() {
    let value: u64 = 0x0123_4567_89AB_CDEF;
    let bytes = encode_u64_le(value);
    assert_eq!(bytes, [0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01]);

    let decoded = decode_u64_le(&bytes).expect("decode failed");
    assert_eq!(decoded, value);

    let (decoded2, consumed) = u64::decode(&bytes).expect("trait decode failed");
    assert_eq!(decoded2, value);
    assert_eq!(consumed, 8);
}

#[test]
fn test_decode_truncated_integers() {
    assert!(matches!(
        decode_u16_le(&[0x01]).unwrap_err(),
        DecodeError::UnexpectedEof
    ));

    assert!(matches!(
        decode_u32_le(&[0x01, 0x02, 0x03]).unwrap_err(),
        DecodeError::UnexpectedEof
    ));

    assert!(matches!(
        decode_u64_le(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]).unwrap_err(),
        DecodeError::UnexpectedEof
    ));
}

#[test]
fn test_encode_decode_array32() {
    let mut value = [0u8; 32];
    for (i, b) in value.iter_mut().enumerate() {
        *b = i as u8;
    }

    let encoded = value.encode();
    assert_eq!(encoded.len(), 32);

    let (decoded, consumed) = <[u8; 32]>::decode(&encoded).expect("decode failed");
    assert_eq!(decoded, value);
    assert_eq!(consumed, 32);
}

#[test]
fn test_encode_decode_vec_small() {
    let value = vec![1u8, 2, 3, 4, 5];
    let encoded = value.encode();

    let (decoded, consumed) = Vec::<u8>::decode(&encoded).expect("decode failed");
    assert_eq!(decoded, value);
    assert_eq!(consumed, encoded.len());
}

#[test]
fn test_vec_varint_length_prefix() {
    let value = vec![0u8; 300];
    let encoded = value.encode();

    let (decoded, consumed) = Vec::<u8>::decode(&encoded).expect("decode failed");
    assert_eq!(decoded.len(), 300);
    assert_eq!(decoded, value);
    assert_eq!(consumed, encoded.len());
}

#[test]
fn test_vec_decode_truncated_body() {
    let value = vec![10u8, 20, 30, 40];
    let encoded = value.encode();
    let truncated = &encoded[..encoded.len() - 1];

    let err = Vec::<u8>::decode(truncated).unwrap_err();
    assert!(matches!(err, DecodeError::UnexpectedEof));
}

#[test]
fn test_vec_roundtrip_various_lengths() {
    for len in [0usize, 1, 10, 252, 253, 10_000] {
        let mut value = Vec::with_capacity(len);
        for i in 0..len {
            value.push((i % 256) as u8);
        }

        let encoded = value.encode();
        let (decoded, consumed) = Vec::<u8>::decode(&encoded).expect("decode failed");
        assert_eq!(decoded, value);
        assert_eq!(consumed, encoded.len());
    }
}

#[test]
fn test_decode_error_invalid_data_from_varint() {
    let bytes = [0xFD, 0x01, 0x00];
    let err = Vec::<u8>::decode(&bytes).unwrap_err();
    assert!(matches!(err, DecodeError::InvalidData));
}
