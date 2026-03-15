use crate::network::protocol::{
    BlockMessage, GetDataMessage, GetHeadersMessage, HeadersMessage, InvMessage, MessagePayload,
    PingMessage, PongMessage, TxMessage, VerackMessage, VersionMessage,
};
use crate::primitives::hash::sha256d;
use crate::primitives::serialize::{Decode, DecodeError, Encode};

pub const MAINNET_MAGIC: [u8; 4] = [0xf9, 0xbe, 0xb4, 0xd9];
pub const TESTNET_MAGIC: [u8; 4] = [0x0b, 0x11, 0x09, 0x07];
pub const REGTEST_MAGIC: [u8; 4] = [0xfa, 0xbf, 0xb5, 0xda];

pub const CMD_INV: &str = "inv";
pub const CMD_GETDATA: &str = "getdata";
pub const CMD_BLOCK: &str = "block";
pub const CMD_GETHEADERS: &str = "getheaders";
pub const CMD_HEADERS: &str = "headers";
pub const CMD_TX: &str = "tx";
pub const CMD_ADDR: &str = "addr";
pub const CMD_GETADDR: &str = "getaddr";

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkMessage {
    pub magic: [u8; 4],
    pub command: [u8; 12],
    pub length: u32,
    pub checksum: [u8; 4],
    pub payload: Vec<u8>,
}

impl NetworkMessage {
    pub fn new(magic: [u8; 4], command_str: &str, payload: Vec<u8>) -> Self {
        let mut command = [0u8; 12];
        let bytes = command_str.as_bytes();
        let len = bytes.len().min(12);
        command[..len].copy_from_slice(&bytes[..len]);

        let hash = sha256d(&payload);
        let mut checksum = [0u8; 4];
        checksum.copy_from_slice(&hash[..4]);

        Self {
            magic,
            command,
            length: payload.len() as u32,
            checksum,
            payload,
        }
    }

    /// Returns `true` if the message's magic bytes match `expected`.
    ///
    /// Callers **must** call this after decoding a `NetworkMessage` received from
    /// the network.  The `Decode` impl intentionally does not validate magic so
    /// that the same parser works for all networks; magic validation is the
    /// responsibility of the connection handler that knows which network it is on.
    pub fn verify_magic(&self, expected: &[u8; 4]) -> bool {
        &self.magic == expected
    }

    pub fn command_string(&self) -> String {
        let mut end = 12;
        for i in 0..12 {
            if self.command[i] == 0 {
                end = i;
                break;
            }
        }
        String::from_utf8_lossy(&self.command[..end]).to_string()
    }

    pub fn parse_payload(&self) -> Result<MessagePayload, DecodeError> {
        let command = self.command_string();
        let bytes = &self.payload;
        match command.as_str() {
            "version" => {
                let (msg, _) = VersionMessage::decode(bytes)?;
                Ok(MessagePayload::Version(msg))
            }
            "verack" => {
                let (msg, _) = VerackMessage::decode(bytes)?;
                Ok(MessagePayload::Verack(msg))
            }
            "ping" => {
                let (msg, _) = PingMessage::decode(bytes)?;
                Ok(MessagePayload::Ping(msg))
            }
            "pong" => {
                let (msg, _) = PongMessage::decode(bytes)?;
                Ok(MessagePayload::Pong(msg))
            }
            "inv" => {
                let (msg, _) = InvMessage::decode(bytes)?;
                Ok(MessagePayload::Inv(msg))
            }
            "getdata" => {
                let (msg, _) = GetDataMessage::decode(bytes)?;
                Ok(MessagePayload::GetData(msg))
            }
            "block" => {
                let (msg, _) = BlockMessage::decode(bytes)?;
                Ok(MessagePayload::Block(msg))
            }
            "getheaders" => {
                let (msg, _) = GetHeadersMessage::decode(bytes)?;
                Ok(MessagePayload::GetHeaders(msg))
            }
            "headers" => {
                let (msg, _) = HeadersMessage::decode(bytes)?;
                Ok(MessagePayload::Headers(msg))
            }
            "tx" => {
                let (msg, _) = TxMessage::decode(bytes)?;
                Ok(MessagePayload::Tx(msg))
            }
            _ => Err(DecodeError::InvalidData),
        }
    }
}

impl Encode for NetworkMessage {
    fn encode(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&self.magic);
        buffer.extend_from_slice(&self.command);
        buffer.extend_from_slice(&self.length.to_le_bytes());
        buffer.extend_from_slice(&self.checksum);
        buffer.extend_from_slice(&self.payload);
        buffer
    }

    fn encoded_size(&self) -> usize {
        24 + self.payload.len()
    }
}

const MAX_PAYLOAD_SIZE: u32 = 32 * 1024 * 1024;

impl Decode for NetworkMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        if bytes.len() < 24 {
            return Err(DecodeError::UnexpectedEof);
        }

        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);

        let mut command = [0u8; 12];
        command.copy_from_slice(&bytes[4..16]);

        let length = u32::from_le_bytes(
            bytes[16..20]
                .try_into()
                .map_err(|_| DecodeError::InvalidData)?,
        );

        if length > MAX_PAYLOAD_SIZE {
            return Err(DecodeError::InvalidData);
        }

        let mut checksum = [0u8; 4];
        checksum.copy_from_slice(&bytes[20..24]);

        let total_len = 24 + length as usize;
        if bytes.len() < total_len {
            return Err(DecodeError::UnexpectedEof);
        }

        let payload = bytes[24..total_len].to_vec();

        // Validate checksum
        let hash = sha256d(&payload);
        if checksum != hash[..4] {
            return Err(DecodeError::InvalidData);
        }

        Ok((
            Self {
                magic,
                command,
                length,
                checksum,
                payload,
            },
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_message_roundtrip() {
        let payload = vec![1, 2, 3, 4, 5];
        let msg = NetworkMessage::new(REGTEST_MAGIC, "test", payload.clone());
        let encoded = msg.encode();
        let (decoded, len) = NetworkMessage::decode(&encoded).unwrap();

        assert_eq!(msg, decoded);
        assert_eq!(len, encoded.len());
        assert_eq!(decoded.magic, REGTEST_MAGIC);
        assert_eq!(decoded.command_string(), "test");
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn test_invalid_checksum() {
        let payload = vec![1, 2, 3];
        let mut msg = NetworkMessage::new(MAINNET_MAGIC, "ping", payload);
        msg.checksum = [0, 0, 0, 0]; // Invalid checksum
        let encoded = msg.encode();
        let result = NetworkMessage::decode(&encoded);
        assert_eq!(result, Err(DecodeError::InvalidData));
    }

    #[test]
    fn test_short_buffer() {
        let bytes = vec![0u8; 23];
        let result = NetworkMessage::decode(&bytes);
        assert_eq!(result, Err(DecodeError::UnexpectedEof));
    }
}
