use crate::consensus::block::BlockHeader;
use crate::consensus::transaction::Transaction;
use crate::primitives::serialize::{Decode, DecodeError, Encode};
use crate::primitives::varint::{decode as decode_varint, encode as encode_varint, VarIntError};
use std::net::SocketAddr;

// Inventory types
pub const INV_TX: u32 = 1;
pub const INV_BLOCK: u32 = 2;

fn map_varint_error(err: VarIntError) -> DecodeError {
    match err {
        VarIntError::UnexpectedEof => DecodeError::UnexpectedEof,
        VarIntError::Overflow => DecodeError::Overflow,
        VarIntError::InvalidPrefix | VarIntError::NonMinimalEncoding => DecodeError::InvalidData,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetAddr {
    pub services: u64,
    pub ip: [u8; 16],
    pub port: u16,
}

impl NetAddr {
    pub fn new(addr: SocketAddr, services: u64) -> Self {
        let (ip, port) = match addr {
            SocketAddr::V4(v4) => {
                let octets = v4.ip().octets();
                let mut ip = [0u8; 16];
                // IPv4-mapped IPv6 address: ::ffff:1.2.3.4
                ip[10] = 0xff;
                ip[11] = 0xff;
                ip[12] = octets[0];
                ip[13] = octets[1];
                ip[14] = octets[2];
                ip[15] = octets[3];
                (ip, v4.port())
            }
            SocketAddr::V6(v6) => (v6.ip().octets(), v6.port()),
        };
        Self { services, ip, port }
    }
}

impl Encode for NetAddr {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.services.encode());
        out.extend_from_slice(&self.ip);
        out.extend_from_slice(&self.port.to_be_bytes()); // Network byte order (Big Endian)
        out
    }

    fn encoded_size(&self) -> usize {
        8 + 16 + 2
    }
}

impl Decode for NetAddr {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        if bytes.len() < 26 {
            return Err(DecodeError::UnexpectedEof);
        }
        let (services, _) = u64::decode(&bytes[0..8])?;
        let mut ip = [0u8; 16];
        ip.copy_from_slice(&bytes[8..24]);
        let port_bytes: [u8; 2] = bytes[24..26].try_into().unwrap();
        let port = u16::from_be_bytes(port_bytes);

        Ok((Self { services, ip, port }, 26))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VersionMessage {
    pub version: u32,
    pub services: u64,
    pub timestamp: i64,
    pub receiver: NetAddr,
    pub sender: NetAddr,
    pub nonce: u64,
    pub user_agent: String,
    pub start_height: u32,
}

impl Encode for VersionMessage {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.version.encode());
        out.extend_from_slice(&self.services.encode());
        out.extend_from_slice(&(self.timestamp as u64).encode());
        out.extend_from_slice(&self.receiver.encode());
        out.extend_from_slice(&self.sender.encode());
        out.extend_from_slice(&self.nonce.encode());
        out.extend_from_slice(&self.user_agent.as_bytes().to_vec().encode());
        out.extend_from_slice(&self.start_height.encode());
        out
    }

    fn encoded_size(&self) -> usize {
        self.version.encoded_size() +
        self.services.encoded_size() +
        8 + // timestamp
        self.receiver.encoded_size() +
        self.sender.encoded_size() +
        self.nonce.encoded_size() +
        self.user_agent.as_bytes().to_vec().encoded_size() +
        self.start_height.encoded_size()
    }
}

impl Decode for VersionMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let mut offset = 0;

        let (version, n) = u32::decode(&bytes[offset..])?;
        offset += n;

        let (services, n) = u64::decode(&bytes[offset..])?;
        offset += n;

        let (timestamp_u64, n) = u64::decode(&bytes[offset..])?;
        let timestamp = timestamp_u64 as i64;
        offset += n;

        let (receiver, n) = NetAddr::decode(&bytes[offset..])?;
        offset += n;

        let (sender, n) = NetAddr::decode(&bytes[offset..])?;
        offset += n;

        let (nonce, n) = u64::decode(&bytes[offset..])?;
        offset += n;

        let (ua_bytes, n) = Vec::<u8>::decode(&bytes[offset..])?;
        offset += n;
        let user_agent = String::from_utf8(ua_bytes).map_err(|_| DecodeError::InvalidData)?;

        let (start_height, n) = u32::decode(&bytes[offset..])?;
        offset += n;

        Ok((
            Self {
                version,
                services,
                timestamp,
                receiver,
                sender,
                nonce,
                user_agent,
                start_height,
            },
            offset,
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PingMessage {
    pub nonce: u64,
}

impl Encode for PingMessage {
    fn encode(&self) -> Vec<u8> {
        self.nonce.encode()
    }

    fn encoded_size(&self) -> usize {
        8
    }
}

impl Decode for PingMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (nonce, n) = u64::decode(bytes)?;
        Ok((Self { nonce }, n))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PongMessage {
    pub nonce: u64,
}

impl Encode for PongMessage {
    fn encode(&self) -> Vec<u8> {
        self.nonce.encode()
    }

    fn encoded_size(&self) -> usize {
        8
    }
}

impl Decode for PongMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (nonce, n) = u64::decode(bytes)?;
        Ok((Self { nonce }, n))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerackMessage;

impl Encode for VerackMessage {
    fn encode(&self) -> Vec<u8> {
        Vec::new()
    }

    fn encoded_size(&self) -> usize {
        0
    }
}

impl Decode for VerackMessage {
    fn decode(_bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        Ok((Self, 0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InvVector {
    pub inv_type: u32,
    pub hash: [u8; 32],
}

impl Encode for InvVector {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.inv_type.encode());
        out.extend_from_slice(&self.hash);
        out
    }

    fn encoded_size(&self) -> usize {
        4 + 32
    }
}

impl Decode for InvVector {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (inv_type, c1) = u32::decode(bytes)?;
        let (hash, c2) = <[u8; 32]>::decode(&bytes[c1..])?;
        Ok((Self { inv_type, hash }, c1 + c2))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InvMessage {
    pub inventory: Vec<InvVector>,
}

impl Encode for InvMessage {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let count = self.inventory.len() as u64;
        out.extend_from_slice(&encode_varint(count));
        for inv in &self.inventory {
            out.extend_from_slice(&inv.encode());
        }
        out
    }

    fn encoded_size(&self) -> usize {
        let mut size = encode_varint(self.inventory.len() as u64).len();
        for inv in &self.inventory {
            size += inv.encoded_size();
        }
        size
    }
}

impl Decode for InvMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (count_u64, mut offset) = decode_varint(bytes).map_err(map_varint_error)?;
        let count: usize = count_u64.try_into().map_err(|_| DecodeError::Overflow)?;

        let mut inventory = Vec::with_capacity(count);
        for _ in 0..count {
            let (inv, consumed) = InvVector::decode(&bytes[offset..])?;
            offset += consumed;
            inventory.push(inv);
        }

        Ok((Self { inventory }, offset))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetDataMessage {
    pub inventory: Vec<InvVector>,
}

impl Encode for GetDataMessage {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let count = self.inventory.len() as u64;
        out.extend_from_slice(&encode_varint(count));
        for inv in &self.inventory {
            out.extend_from_slice(&inv.encode());
        }
        out
    }

    fn encoded_size(&self) -> usize {
        let mut size = encode_varint(self.inventory.len() as u64).len();
        for inv in &self.inventory {
            size += inv.encoded_size();
        }
        size
    }
}

impl Decode for GetDataMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (count_u64, mut offset) = decode_varint(bytes).map_err(map_varint_error)?;
        let count: usize = count_u64.try_into().map_err(|_| DecodeError::Overflow)?;

        let mut inventory = Vec::with_capacity(count);
        for _ in 0..count {
            let (inv, consumed) = InvVector::decode(&bytes[offset..])?;
            offset += consumed;
            inventory.push(inv);
        }

        Ok((Self { inventory }, offset))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockMessage {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
}

impl Encode for BlockMessage {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.header.encode());

        let count = self.transactions.len() as u64;
        out.extend_from_slice(&encode_varint(count));

        for tx in &self.transactions {
            out.extend_from_slice(&tx.encode());
        }
        out
    }

    fn encoded_size(&self) -> usize {
        let mut size =
            self.header.encoded_size() + encode_varint(self.transactions.len() as u64).len();
        for tx in &self.transactions {
            size += tx.encoded_size();
        }
        size
    }
}

impl Decode for BlockMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (header, mut offset) = BlockHeader::decode(bytes)?;

        let (count_u64, consumed) = decode_varint(&bytes[offset..]).map_err(map_varint_error)?;
        offset += consumed;

        let count: usize = count_u64.try_into().map_err(|_| DecodeError::Overflow)?;
        let mut transactions = Vec::with_capacity(count);

        for _ in 0..count {
            let (tx, consumed) = Transaction::decode(&bytes[offset..])?;
            offset += consumed;
            transactions.push(tx);
        }

        Ok((
            Self {
                header,
                transactions,
            },
            offset,
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetHeadersMessage {
    pub version: u32,
    pub block_locator: Vec<[u8; 32]>,
    pub stop_hash: [u8; 32],
}

impl Encode for GetHeadersMessage {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.version.encode());

        let count = self.block_locator.len() as u64;
        out.extend_from_slice(&encode_varint(count));

        for hash in &self.block_locator {
            out.extend_from_slice(hash);
        }

        out.extend_from_slice(&self.stop_hash);
        out
    }

    fn encoded_size(&self) -> usize {
        let mut size = 4; // version
        size += encode_varint(self.block_locator.len() as u64).len();
        size += self.block_locator.len() * 32;
        size += 32; // stop_hash
        size
    }
}

impl Decode for GetHeadersMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let mut offset = 0;
        let (version, consumed) = u32::decode(&bytes[offset..])?;
        offset += consumed;

        let (count_u64, consumed) = decode_varint(&bytes[offset..]).map_err(map_varint_error)?;
        offset += consumed;

        let count: usize = count_u64.try_into().map_err(|_| DecodeError::Overflow)?;
        let mut block_locator = Vec::with_capacity(count);

        for _ in 0..count {
            let (hash, consumed) = <[u8; 32]>::decode(&bytes[offset..])?;
            offset += consumed;
            block_locator.push(hash);
        }

        let (stop_hash, consumed) = <[u8; 32]>::decode(&bytes[offset..])?;
        offset += consumed;

        Ok((
            Self {
                version,
                block_locator,
                stop_hash,
            },
            offset,
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeadersMessage {
    pub headers: Vec<BlockHeader>,
}

impl Encode for HeadersMessage {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();

        let count = self.headers.len() as u64;
        out.extend_from_slice(&encode_varint(count));

        for header in &self.headers {
            out.extend_from_slice(&header.encode());
            // Bitcoin protocol includes a transaction count of 0 after each header in headers message
            out.extend_from_slice(&encode_varint(0));
        }
        out
    }

    fn encoded_size(&self) -> usize {
        let mut size = encode_varint(self.headers.len() as u64).len();
        for header in &self.headers {
            size += header.encoded_size();
            size += 1; // varint(0) takes 1 byte
        }
        size
    }
}

impl Decode for HeadersMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (count_u64, mut offset) = decode_varint(bytes).map_err(map_varint_error)?;
        let count: usize = count_u64.try_into().map_err(|_| DecodeError::Overflow)?;

        let mut headers = Vec::with_capacity(count);

        for _ in 0..count {
            let (header, consumed) = BlockHeader::decode(&bytes[offset..])?;
            offset += consumed;

            // Read and verify transaction count is 0
            let (tx_count, consumed) = decode_varint(&bytes[offset..]).map_err(map_varint_error)?;
            if tx_count != 0 {
                // In a headers message, tx count should be 0
                // Some implementations might send partial blocks?
                // Bitcoin protocol spec says: "The block headers are sent in a vector... For each block header, a txn_count is sent... It must be 0."
                // But let's be strict for now or just consume it.
            }
            offset += consumed;

            headers.push(header);
        }

        Ok((Self { headers }, offset))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TxMessage {
    pub transaction: Transaction,
}

impl Encode for TxMessage {
    fn encode(&self) -> Vec<u8> {
        self.transaction.encode()
    }

    fn encoded_size(&self) -> usize {
        self.transaction.encoded_size()
    }
}

impl Decode for TxMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (transaction, consumed) = Transaction::decode(bytes)?;
        Ok((Self { transaction }, consumed))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkAddr {
    pub timestamp: u32,
    pub services: u64,
    pub ip: [u8; 16],
    pub port: u16,
}

impl Encode for NetworkAddr {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.timestamp.encode());
        out.extend_from_slice(&self.services.encode());
        out.extend_from_slice(&self.ip);
        out.extend_from_slice(&self.port.to_be_bytes());
        out
    }

    fn encoded_size(&self) -> usize {
        4 + 8 + 16 + 2
    }
}

impl Decode for NetworkAddr {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let mut offset = 0;
        let (timestamp, consumed) = u32::decode(&bytes[offset..])?;
        offset += consumed;

        let (services, consumed) = u64::decode(&bytes[offset..])?;
        offset += consumed;

        if bytes.len() < offset + 18 {
            return Err(DecodeError::UnexpectedEof);
        }

        let mut ip = [0u8; 16];
        ip.copy_from_slice(&bytes[offset..offset + 16]);
        offset += 16;

        let port_bytes: [u8; 2] = bytes[offset..offset + 2].try_into().unwrap();
        let port = u16::from_be_bytes(port_bytes);
        offset += 2;

        Ok((
            Self {
                timestamp,
                services,
                ip,
                port,
            },
            offset,
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AddrMessage {
    pub addresses: Vec<NetworkAddr>,
}

impl Encode for AddrMessage {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let count = self.addresses.len() as u64;
        out.extend_from_slice(&encode_varint(count));
        for addr in &self.addresses {
            out.extend_from_slice(&addr.encode());
        }
        out
    }

    fn encoded_size(&self) -> usize {
        let mut size = encode_varint(self.addresses.len() as u64).len();
        for addr in &self.addresses {
            size += addr.encoded_size();
        }
        size
    }
}

impl Decode for AddrMessage {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (count_u64, mut offset) = decode_varint(bytes).map_err(map_varint_error)?;
        let count: usize = count_u64.try_into().map_err(|_| DecodeError::Overflow)?;

        let mut addresses = Vec::with_capacity(count);
        for _ in 0..count {
            let (addr, consumed) = NetworkAddr::decode(&bytes[offset..])?;
            offset += consumed;
            addresses.push(addr);
        }

        Ok((Self { addresses }, offset))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetAddrMessage;

impl Encode for GetAddrMessage {
    fn encode(&self) -> Vec<u8> {
        Vec::new()
    }

    fn encoded_size(&self) -> usize {
        0
    }
}

impl Decode for GetAddrMessage {
    fn decode(_bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        Ok((Self, 0))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessagePayload {
    Version(VersionMessage),
    Verack(VerackMessage),
    Ping(PingMessage),
    Pong(PongMessage),
    Inv(InvMessage),
    GetData(GetDataMessage),
    Block(BlockMessage),
    GetHeaders(GetHeadersMessage),
    Headers(HeadersMessage),
    Tx(TxMessage),
    Addr(AddrMessage),
    GetAddr(GetAddrMessage),
}

impl MessagePayload {
    pub fn command(&self) -> &'static str {
        match self {
            MessagePayload::Version(_) => "version",
            MessagePayload::Verack(_) => "verack",
            MessagePayload::Ping(_) => "ping",
            MessagePayload::Pong(_) => "pong",
            MessagePayload::Inv(_) => "inv",
            MessagePayload::GetData(_) => "getdata",
            MessagePayload::Block(_) => "block",
            MessagePayload::GetHeaders(_) => "getheaders",
            MessagePayload::Headers(_) => "headers",
            MessagePayload::Tx(_) => "tx",
            MessagePayload::Addr(_) => "addr",
            MessagePayload::GetAddr(_) => "getaddr",
        }
    }
}

impl Encode for MessagePayload {
    fn encode(&self) -> Vec<u8> {
        match self {
            MessagePayload::Version(msg) => msg.encode(),
            MessagePayload::Verack(msg) => msg.encode(),
            MessagePayload::Ping(msg) => msg.encode(),
            MessagePayload::Pong(msg) => msg.encode(),
            MessagePayload::Inv(msg) => msg.encode(),
            MessagePayload::GetData(msg) => msg.encode(),
            MessagePayload::Block(msg) => msg.encode(),
            MessagePayload::GetHeaders(msg) => msg.encode(),
            MessagePayload::Headers(msg) => msg.encode(),
            MessagePayload::Tx(msg) => msg.encode(),
            MessagePayload::Addr(msg) => msg.encode(),
            MessagePayload::GetAddr(msg) => msg.encode(),
        }
    }

    fn encoded_size(&self) -> usize {
        match self {
            MessagePayload::Version(msg) => msg.encoded_size(),
            MessagePayload::Verack(msg) => msg.encoded_size(),
            MessagePayload::Ping(msg) => msg.encoded_size(),
            MessagePayload::Pong(msg) => msg.encoded_size(),
            MessagePayload::Inv(msg) => msg.encoded_size(),
            MessagePayload::GetData(msg) => msg.encoded_size(),
            MessagePayload::Block(msg) => msg.encoded_size(),
            MessagePayload::GetHeaders(msg) => msg.encoded_size(),
            MessagePayload::Headers(msg) => msg.encoded_size(),
            MessagePayload::Tx(msg) => msg.encoded_size(),
            MessagePayload::Addr(msg) => msg.encoded_size(),
            MessagePayload::GetAddr(msg) => msg.encoded_size(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_netaddr_encode_decode() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8333);
        let net_addr = NetAddr::new(addr, 1);
        let encoded = net_addr.encode();
        let (decoded, len) = NetAddr::decode(&encoded).unwrap();
        assert_eq!(net_addr, decoded);
        assert_eq!(len, encoded.len());
    }

    #[test]
    fn test_version_message_roundtrip() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8333);
        let net_addr = NetAddr::new(addr, 1);

        let msg = VersionMessage {
            version: 70015,
            services: 1,
            timestamp: 123456789,
            receiver: net_addr.clone(),
            sender: net_addr,
            nonce: 0x1234567890abcdef,
            user_agent: "/Satoshi:0.18.0/".to_string(),
            start_height: 100,
        };

        let encoded = msg.encode();
        let (decoded, len) = VersionMessage::decode(&encoded).unwrap();

        assert_eq!(msg, decoded);
        assert_eq!(len, encoded.len());
    }

    #[test]
    fn test_ping_pong_roundtrip() {
        let ping = PingMessage { nonce: 12345 };
        let encoded = ping.encode();
        let (decoded, len) = PingMessage::decode(&encoded).unwrap();
        assert_eq!(ping, decoded);
        assert_eq!(len, encoded.len());

        let pong = PongMessage { nonce: 67890 };
        let encoded = pong.encode();
        let (decoded, len) = PongMessage::decode(&encoded).unwrap();
        assert_eq!(pong, decoded);
        assert_eq!(len, encoded.len());
    }

    #[test]
    fn test_inv_message_roundtrip() {
        let inv1 = InvVector {
            inv_type: INV_TX,
            hash: [0x11; 32],
        };
        let inv2 = InvVector {
            inv_type: INV_BLOCK,
            hash: [0x22; 32],
        };
        let msg = InvMessage {
            inventory: vec![inv1, inv2],
        };

        let encoded = msg.encode();
        let (decoded, len) = InvMessage::decode(&encoded).unwrap();

        assert_eq!(msg, decoded);
        assert_eq!(len, encoded.len());
    }

    #[test]
    fn test_getdata_message_roundtrip() {
        let inv = InvVector {
            inv_type: INV_BLOCK,
            hash: [0x33; 32],
        };
        let msg = GetDataMessage {
            inventory: vec![inv],
        };

        let encoded = msg.encode();
        let (decoded, len) = GetDataMessage::decode(&encoded).unwrap();

        assert_eq!(msg, decoded);
        assert_eq!(len, encoded.len());
    }

    #[test]
    fn test_block_message_roundtrip() {
        let header = BlockHeader {
            version: 1,
            prev_block_hash: [0x44; 32],
            merkle_root: [0x55; 32],
            timestamp: 123456,
            n_bits: 0x1d00ffff,
            nonce: 0,
        };
        // Empty transactions for simplicity, assuming Transaction roundtrip works
        let msg = BlockMessage {
            header,
            transactions: vec![],
        };

        let encoded = msg.encode();
        let (decoded, len) = BlockMessage::decode(&encoded).unwrap();

        assert_eq!(msg, decoded);
        assert_eq!(len, encoded.len());
    }
}
