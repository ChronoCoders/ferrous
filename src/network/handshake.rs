use crate::network::peer::Peer;
use crate::network::protocol::VersionMessage;
use crate::primitives::serialize::Decode;
use std::thread;
use std::time::{Duration, Instant};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

pub fn perform_handshake(
    peer: &mut Peer,
    our_version: u32,
    our_services: u64,
    our_height: u32,
    our_nonce: u64,
) -> Result<(), String> {
    // 1. Send Version
    peer.initiate_handshake(our_version, our_services, our_height, our_nonce)?;

    // 2. Wait for Version
    let start = Instant::now();
    loop {
        if start.elapsed() > HANDSHAKE_TIMEOUT {
            return Err("Handshake timeout waiting for version".to_string());
        }

        match peer.receive() {
            Ok(Some(msg)) => {
                if msg.command_string() == "version" {
                    let (version_msg, _) = VersionMessage::decode(&msg.payload)
                        .map_err(|e| format!("Failed to decode version: {:?}", e))?;

                    if version_msg.nonce == our_nonce {
                        return Err("Connected to self".to_string());
                    }

                    peer.handle_version(&version_msg)?;
                    break;
                } else {
                    // Ignore other messages or error?
                    // For handshake, we expect version first.
                    return Err(format!("Expected version, got {}", msg.command_string()));
                }
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }

    // 3. Send Verack (handled in handle_version mostly, but explicit send needed?)
    // handle_version sends verack.

    // 4. Wait for Verack
    loop {
        if start.elapsed() > HANDSHAKE_TIMEOUT {
            return Err("Handshake timeout waiting for verack".to_string());
        }

        match peer.receive() {
            Ok(Some(msg)) => {
                if msg.command_string() == "verack" {
                    peer.handle_verack()?;
                    break;
                } else if msg.command_string() == "version" {
                    // Duplicate version? Error.
                    return Err("Received duplicate version".to_string());
                } else {
                    // Ignore or buffer? For strict handshake, error.
                    // But ping might come?
                    if msg.command_string() == "ping" {
                        continue;
                    }
                    return Err(format!("Expected verack, got {}", msg.command_string()));
                }
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

pub fn perform_inbound_handshake(
    peer: &mut Peer,
    our_version: u32,
    our_services: u64,
    our_height: u32,
    our_nonce: u64,
) -> Result<(), String> {
    let start = Instant::now();

    // 1. Wait for Version from peer
    let version_msg = loop {
        if start.elapsed() > HANDSHAKE_TIMEOUT {
            return Err("Handshake timeout waiting for version".to_string());
        }

        match peer.receive() {
            Ok(Some(msg)) => {
                if msg.command_string() == "version" {
                    let (version_msg, _) = VersionMessage::decode(&msg.payload)
                        .map_err(|e| format!("Failed to decode version: {:?}", e))?;

                    if version_msg.nonce == our_nonce {
                        return Err("Connected to self".to_string());
                    }

                    break version_msg;
                } else {
                    return Err(format!("Expected version, got {}", msg.command_string()));
                }
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    };

    // 2. Send our Version first (so peer receives Version before Verack)
    peer.initiate_handshake(our_version, our_services, our_height, our_nonce)?;

    // 3. Now handle peer's Version (sends Verack)
    peer.handle_version(&version_msg)?;

    // 4. Wait for Verack
    loop {
        if start.elapsed() > HANDSHAKE_TIMEOUT {
            return Err("Handshake timeout waiting for verack".to_string());
        }

        match peer.receive() {
            Ok(Some(msg)) => {
                if msg.command_string() == "verack" {
                    peer.handle_verack()?;
                    break;
                } else if msg.command_string() == "ping" {
                    continue;
                } else {
                    return Err(format!("Expected verack, got {}", msg.command_string()));
                }
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::connection::PeerConnection;
    use crate::network::message::REGTEST_MAGIC;
    use std::net::{TcpListener, TcpStream};

    const PROTOCOL_VERSION: u32 = 70015;

    fn create_test_peer_pair() -> (Peer, Peer) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            PeerConnection::new(stream, REGTEST_MAGIC).unwrap()
        });

        let stream = TcpStream::connect(addr).unwrap();
        let client_conn = PeerConnection::new(stream, REGTEST_MAGIC).unwrap();
        let server_conn = handle.join().unwrap();

        let client_peer = Peer::new_inbound(1, client_conn);
        let server_peer = Peer::new_inbound(2, server_conn);

        (client_peer, server_peer)
    }

    #[test]
    fn test_handshake_success() {
        let (mut client, mut server) = create_test_peer_pair();

        let client_thread =
            thread::spawn(move || perform_handshake(&mut client, PROTOCOL_VERSION, 0, 0, 12345));

        let server_thread =
            thread::spawn(move || perform_handshake(&mut server, PROTOCOL_VERSION, 0, 0, 67890));

        assert!(client_thread.join().unwrap().is_ok());
        assert!(server_thread.join().unwrap().is_ok());
    }

    #[test]
    fn test_self_connection() {
        let (mut client, mut server) = create_test_peer_pair();

        let nonce = 12345;

        let client_thread =
            thread::spawn(move || perform_handshake(&mut client, PROTOCOL_VERSION, 0, 0, nonce));

        let server_thread =
            thread::spawn(move || perform_handshake(&mut server, PROTOCOL_VERSION, 0, 0, nonce));

        let res1 = client_thread.join().unwrap();
        let res2 = server_thread.join().unwrap();

        // One or both should detect self-connection
        assert!(res1.is_err() || res2.is_err());
        if let Err(e) = res1 {
            assert_eq!(e, "Connected to self");
        }
    }
}
