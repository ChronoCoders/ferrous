use crate::network::connection::PeerConnection;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct NetworkListener {
    addr: SocketAddr,
    magic: [u8; 4],
    max_peers: usize,
    active_peers: Arc<Mutex<usize>>,
}

impl NetworkListener {
    pub fn new(addr: SocketAddr, magic: [u8; 4], max_peers: usize) -> Self {
        Self {
            addr,
            magic,
            max_peers,
            active_peers: Arc::new(Mutex::new(0)),
        }
    }

    pub fn start<F>(&self, on_peer: F) -> Result<(), String>
    where
        F: Fn(PeerConnection) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(self.addr)
            .map_err(|e| format!("Failed to bind to address {}: {}", self.addr, e))?;

        let magic = self.magic;
        let max_peers = self.max_peers;
        let active_peers = Arc::clone(&self.active_peers);
        let on_peer = Arc::new(on_peer);

        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let mut count = active_peers.lock().unwrap();
                        if *count >= max_peers {
                            // Too many peers, drop connection
                            continue;
                        }
                        *count += 1;
                        drop(count); // Release lock

                        let active_peers_clone = Arc::clone(&active_peers);
                        let on_peer_clone = Arc::clone(&on_peer);

                        thread::spawn(move || {
                            match PeerConnection::new(stream, magic) {
                                Ok(conn) => {
                                    on_peer_clone(conn);
                                }
                                Err(e) => {
                                    eprintln!("Failed to create peer connection: {}", e);
                                }
                            }
                            // Decrement count when peer disconnects (handled by on_peer returning)
                            let mut count = active_peers_clone.lock().unwrap();
                            *count -= 1;
                        });
                    }
                    Err(e) => {
                        eprintln!("Error accepting connection: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    pub fn bind(&self) -> Result<TcpListener, String> {
        TcpListener::bind(self.addr)
            .map_err(|e| format!("Failed to bind to address {}: {}", self.addr, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::message::REGTEST_MAGIC;
    use std::net::TcpStream;
    use std::time::Duration;

    #[test]
    fn test_listener_accepts_connections() {
        let listener = NetworkListener::new("127.0.0.1:0".parse().unwrap(), REGTEST_MAGIC, 5);
        let (tx, _rx) = std::sync::mpsc::channel();

        listener
            .start(move |conn| {
                tx.send(conn.peer_addr()).unwrap();
            })
            .unwrap();

        // Wait for listener to start (hacky but simple)
        thread::sleep(Duration::from_millis(100));

        // Since we bound to port 0, we can't easily know the port to connect to.
        // So for this test, we'll use bind() directly to get the port first.
        let tcp_listener = listener.bind().unwrap();
        let addr = tcp_listener.local_addr().unwrap();

        // Drop the temporary listener to free the port
        drop(tcp_listener);
        drop(listener);

        // Re-create listener with known port for the test proper
        let listener = NetworkListener::new(addr, REGTEST_MAGIC, 5);
        let (tx, rx) = std::sync::mpsc::channel();

        listener
            .start(move |_| {
                tx.send(true).unwrap();
            })
            .unwrap();

        TcpStream::connect(addr).unwrap();

        // Check if callback was called
        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap());
    }
}
