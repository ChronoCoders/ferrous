use crate::primitives::varint::{decode as decode_varint, encode as encode_varint, VarIntError};

/// Error type returned by decoding operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The input ended unexpectedly while decoding a value.
    UnexpectedEof,
    /// The input data is malformed or violates encoding rules.
    InvalidData,
    /// The decoded value exceeds the supported numeric range.
    Overflow,
}

/// Trait for types that can be encoded into a byte vector.
pub trait Encode {
    /// Encodes the value into its binary representation.
    fn encode(&self) -> Vec<u8>;

    /// Returns the size in bytes of the encoded representation.
    fn encoded_size(&self) -> usize;
}

/// Trait for types that can be decoded from a byte slice.
pub trait Decode: Sized {
    /// Decodes a value from the beginning of the given byte slice.
    ///
    /// On success, returns the decoded value together with the number of bytes
    /// consumed from the input slice.
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError>;
}

/// Encodes a 16-bit unsigned integer in little-endian order.
pub fn encode_u16_le(value: u16) -> [u8; 2] {
    value.to_le_bytes()
}

/// Encodes a 32-bit unsigned integer in little-endian order.
pub fn encode_u32_le(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

/// Encodes a 64-bit unsigned integer in little-endian order.
pub fn encode_u64_le(value: u64) -> [u8; 8] {
    value.to_le_bytes()
}

/// Decodes a 16-bit unsigned integer from little-endian bytes.
pub fn decode_u16_le(bytes: &[u8]) -> Result<u16, DecodeError> {
    if bytes.len() < 2 {
        return Err(DecodeError::UnexpectedEof);
    }

    let mut buf = [0u8; 2];
    buf.copy_from_slice(&bytes[..2]);
    Ok(u16::from_le_bytes(buf))
}

/// Decodes a 32-bit unsigned integer from little-endian bytes.
pub fn decode_u32_le(bytes: &[u8]) -> Result<u32, DecodeError> {
    if bytes.len() < 4 {
        return Err(DecodeError::UnexpectedEof);
    }

    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes[..4]);
    Ok(u32::from_le_bytes(buf))
}

/// Decodes a 64-bit unsigned integer from little-endian bytes.
pub fn decode_u64_le(bytes: &[u8]) -> Result<u64, DecodeError> {
    if bytes.len() < 8 {
        return Err(DecodeError::UnexpectedEof);
    }

    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[..8]);
    Ok(u64::from_le_bytes(buf))
}

impl Encode for u8 {
    fn encode(&self) -> Vec<u8> {
        vec![*self]
    }

    fn encoded_size(&self) -> usize {
        1
    }
}

impl Decode for u8 {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let byte = *bytes.first().ok_or(DecodeError::UnexpectedEof)?;
        Ok((byte, 1))
    }
}

impl Encode for u16 {
    fn encode(&self) -> Vec<u8> {
        encode_u16_le(*self).to_vec()
    }

    fn encoded_size(&self) -> usize {
        2
    }
}

impl Decode for u16 {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let value = decode_u16_le(bytes)?;
        Ok((value, 2))
    }
}

impl Encode for u32 {
    fn encode(&self) -> Vec<u8> {
        encode_u32_le(*self).to_vec()
    }

    fn encoded_size(&self) -> usize {
        4
    }
}

impl Decode for u32 {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let value = decode_u32_le(bytes)?;
        Ok((value, 4))
    }
}

impl Encode for u64 {
    fn encode(&self) -> Vec<u8> {
        encode_u64_le(*self).to_vec()
    }

    fn encoded_size(&self) -> usize {
        8
    }
}

impl Decode for u64 {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let value = decode_u64_le(bytes)?;
        Ok((value, 8))
    }
}

impl Encode for [u8; 32] {
    fn encode(&self) -> Vec<u8> {
        self.to_vec()
    }

    fn encoded_size(&self) -> usize {
        32
    }
}

impl Decode for [u8; 32] {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        if bytes.len() < 32 {
            return Err(DecodeError::UnexpectedEof);
        }

        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes[..32]);
        Ok((out, 32))
    }
}

impl Encode for Vec<u8> {
    fn encode(&self) -> Vec<u8> {
        let len = self.len() as u64;
        let mut out = encode_varint(len);
        out.extend_from_slice(self);
        out
    }

    fn encoded_size(&self) -> usize {
        encode_varint(self.len() as u64).len() + self.len()
    }
}

impl Decode for Vec<u8> {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (len_u64, consumed) = decode_varint(bytes).map_err(map_varint_error)?;

        let len_usize: usize = len_u64.try_into().map_err(|_| DecodeError::Overflow)?;

        if bytes.len() < consumed + len_usize {
            return Err(DecodeError::UnexpectedEof);
        }

        let data = bytes[consumed..consumed + len_usize].to_vec();
        Ok((data, consumed + len_usize))
    }
}

fn map_varint_error(err: VarIntError) -> DecodeError {
    match err {
        VarIntError::UnexpectedEof => DecodeError::UnexpectedEof,
        VarIntError::Overflow => DecodeError::Overflow,
        VarIntError::InvalidPrefix | VarIntError::NonMinimalEncoding => DecodeError::InvalidData,
    }
}
