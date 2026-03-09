use crate::network::message::NetworkMessage;
use crate::primitives::serialize::{Decode, Encode};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const MAX_MESSAGE_SIZE: u32 = 32 * 1024 * 1024; // 32MB
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct PeerConnection {
    stream: TcpStream,
    magic: [u8; 4],
    addr: SocketAddr,
    buffer: Vec<u8>,
}

impl PeerConnection {
    pub fn new(stream: TcpStream, magic: [u8; 4]) -> Result<Self, String> {
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .map_err(|e| format!("Failed to set read timeout: {}", e))?;
        stream
            .set_write_timeout(Some(WRITE_TIMEOUT))
            .map_err(|e| format!("Failed to set write timeout: {}", e))?;

        let addr = stream
            .peer_addr()
            .map_err(|e| format!("Failed to get peer address: {}", e))?;

        Ok(Self {
            stream,
            magic,
            addr,
            buffer: Vec::new(),
        })
    }

    pub fn send_message(&mut self, message: &NetworkMessage) -> Result<(), String> {
        let encoded = message.encode();
        self.stream
            .write_all(&encoded)
            .map_err(|e| format!("Failed to send message: {}", e))
    }

    // Blocking read until a full message is available
    pub fn read_message(&mut self) -> Result<NetworkMessage, String> {
        loop {
            // Check if we already have a full message in buffer
            if let Some((msg, consumed)) = self.try_parse_from_buffer()? {
                // Remove consumed bytes
                self.buffer.drain(0..consumed);
                return Ok(msg);
            }

            // If not, read more data from stream (blocking)
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err("Connection closed".to_string()),
                Ok(n) => {
                    self.buffer.extend_from_slice(&chunk[..n]);
                }
                Err(e) => return Err(format!("Read error: {}", e)),
            }
        }
    }

    // Non-blocking read attempt
    pub fn try_read_message(&mut self) -> Result<Option<NetworkMessage>, String> {
        // First, try to read any available data from the stream into our buffer
        self.stream
            .set_nonblocking(true)
            .map_err(|e| format!("Failed to set nonblocking mode: {}", e))?;

        let mut chunk = [0u8; 4096];
        match self.stream.read(&mut chunk) {
            Ok(0) => {
                // Connection closed by peer?
                // For try_read, this might mean EOF.
                // If buffer is empty, return None (or error?).
                // Usually try_read returns None if no message.
                // But if EOF, connection is dead.
                // We'll return None here, but ideally we should signal disconnect.
                // But for now, let's just return None and let subsequent calls fail or handle it.
            }
            Ok(n) => {
                self.buffer.extend_from_slice(&chunk[..n]);
            }
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                // No data available right now, continue
            }
            Err(e) => return Err(format!("Read error: {}", e)),
        }

        self.stream
            .set_nonblocking(false)
            .map_err(|e| format!("Failed to set blocking mode: {}", e))?;

        // Now try to parse a message from the buffer
        if let Some((msg, consumed)) = self.try_parse_from_buffer()? {
            self.buffer.drain(0..consumed);
            Ok(Some(msg))
        } else {
            Ok(None)
        }
    }

    fn try_parse_from_buffer(&self) -> Result<Option<(NetworkMessage, usize)>, String> {
        if self.buffer.len() < 24 {
            return Ok(None);
        }

        // Check magic
        if self.buffer[0..4] != self.magic {
            return Err(format!(
                "Invalid magic bytes: expected {:?}, got {:?}",
                self.magic,
                &self.buffer[0..4]
            ));
        }

        // Parse length
        let length_bytes: [u8; 4] = self.buffer[16..20].try_into().unwrap();
        let length = u32::from_le_bytes(length_bytes);

        if length > MAX_MESSAGE_SIZE {
            return Err(format!(
                "Message too large: {} bytes (max {})",
                length, MAX_MESSAGE_SIZE
            ));
        }

        let total_len = 24 + length as usize;
        if self.buffer.len() < total_len {
            return Ok(None); // Need more data
        }

        // Decode message
        match NetworkMessage::decode(&self.buffer[..total_len]) {
            Ok((msg, consumed)) => Ok(Some((msg, consumed))),
            Err(e) => Err(format!("Failed to decode message: {:?}", e)),
        }
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn magic(&self) -> [u8; 4] {
        self.magic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::message::REGTEST_MAGIC;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn test_peer_connection_send_receive() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut conn = PeerConnection::new(stream, REGTEST_MAGIC).unwrap();
            let msg = conn.read_message().unwrap();
            conn.send_message(&msg).unwrap();
        });

        let stream = TcpStream::connect(addr).unwrap();
        let mut conn = PeerConnection::new(stream, REGTEST_MAGIC).unwrap();

        let payload = vec![1, 2, 3, 4];
        let msg = NetworkMessage::new(REGTEST_MAGIC, "ping", payload);
        conn.send_message(&msg).unwrap();

        server_thread.join().unwrap();

        let response = conn.read_message().unwrap();
        assert_eq!(msg, response);
    }

    #[test]
    fn test_invalid_magic() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Send garbage magic
            let mut msg = NetworkMessage::new(REGTEST_MAGIC, "ping", vec![]);
            msg.magic = [0xde, 0xad, 0xbe, 0xef];
            let encoded = msg.encode();
            stream.write_all(&encoded).unwrap();
        });

        let stream = TcpStream::connect(addr).unwrap();
        let mut conn = PeerConnection::new(stream, REGTEST_MAGIC).unwrap(); // Expects REGTEST_MAGIC

        let result = conn.read_message();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid magic bytes"));
    }
}
