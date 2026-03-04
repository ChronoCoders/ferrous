/// Variable-length integer encoded using the Bitcoin-style VarInt format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarInt(pub u64);

/// Errors that can occur while decoding a VarInt value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarIntError {
    /// The prefix byte is not a valid VarInt discriminator.
    InvalidPrefix,
    /// The value is not encoded using the minimal possible representation.
    NonMinimalEncoding,
    /// The input ended unexpectedly while decoding a value.
    UnexpectedEof,
    /// The decoded value exceeds the supported numeric range.
    Overflow,
}

/// Encodes an unsigned 64-bit integer into its VarInt byte representation.
pub fn encode(value: u64) -> Vec<u8> {
    if value < 0xFD {
        vec![value as u8]
    } else if value <= 0xFFFF {
        let mut out = Vec::with_capacity(3);
        out.push(0xFD);
        out.extend_from_slice(&(value as u16).to_le_bytes());
        out
    } else if value <= 0xFFFF_FFFF {
        let mut out = Vec::with_capacity(5);
        out.push(0xFE);
        out.extend_from_slice(&(value as u32).to_le_bytes());
        out
    } else {
        let mut out = Vec::with_capacity(9);
        out.push(0xFF);
        out.extend_from_slice(&value.to_le_bytes());
        out
    }
}

/// Decodes a VarInt-encoded value from the given byte slice.
///
/// On success, returns the decoded value together with the number of bytes
/// consumed from the input slice.
pub fn decode(bytes: &[u8]) -> Result<(u64, usize), VarIntError> {
    let prefix = *bytes.first().ok_or(VarIntError::UnexpectedEof)?;

    match prefix {
        0x00..=0xFC => Ok((prefix as u64, 1)),
        0xFD => {
            if bytes.len() < 3 {
                return Err(VarIntError::UnexpectedEof);
            }

            let mut buf = [0u8; 2];
            buf.copy_from_slice(&bytes[1..3]);
            let value = u16::from_le_bytes(buf) as u64;

            if value < 0xFD {
                return Err(VarIntError::NonMinimalEncoding);
            }

            Ok((value, 3))
        }
        0xFE => {
            if bytes.len() < 5 {
                return Err(VarIntError::UnexpectedEof);
            }

            let mut buf = [0u8; 4];
            buf.copy_from_slice(&bytes[1..5]);
            let value = u32::from_le_bytes(buf) as u64;

            if value <= 0xFFFF {
                return Err(VarIntError::NonMinimalEncoding);
            }

            Ok((value, 5))
        }
        0xFF => {
            if bytes.len() < 9 {
                return Err(VarIntError::UnexpectedEof);
            }

            let mut buf = [0u8; 8];
            buf.copy_from_slice(&bytes[1..9]);
            let value = u64::from_le_bytes(buf);

            if value <= 0xFFFF_FFFF {
                return Err(VarIntError::NonMinimalEncoding);
            }

            Ok((value, 9))
        }
    }
}
