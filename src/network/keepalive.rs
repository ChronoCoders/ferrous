use crate::network::manager::PeerManager;
use crate::network::protocol::PingMessage;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub struct KeepaliveManager {
    peer_manager: Arc<PeerManager>,
    ping_state: Arc<Mutex<HashMap<u64, PingState>>>,
}

struct PingState {
    last_ping_sent: Instant,
    #[allow(dead_code)] // Field is read but not used in logic yet (future use for latency calc)
    last_pong_received: Instant,
    pending_nonce: Option<u64>,
    missed_pongs: u32,
}

impl KeepaliveManager {
    pub fn new(peer_manager: Arc<PeerManager>) -> Self {
        Self {
            peer_manager,
            ping_state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start(&self) {
        let peer_manager = self.peer_manager.clone();
        let ping_state = self.ping_state.clone();

        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(60)); // Ping every minute

                let peer_ids = peer_manager.get_active_peer_ids();

                for peer_id in peer_ids {
                    let mut state = ping_state.lock().unwrap();

                    let ping_state = state.entry(peer_id).or_insert(PingState {
                        last_ping_sent: Instant::now(),
                        last_pong_received: Instant::now(),
                        pending_nonce: None,
                        missed_pongs: 0,
                    });

                    // Check if previous ping was answered
                    if let Some(_pending) = ping_state.pending_nonce {
                        // Pong not received for previous ping
                        ping_state.missed_pongs += 1;

                        if ping_state.missed_pongs >= 3 {
                            println!("Peer {} missed 3 pongs, disconnecting", peer_id);
                            drop(state); // Drop lock before calling into peer_manager to avoid potential deadlocks
                            let _ = peer_manager.disconnect_peer(peer_id);
                            continue;
                        }
                    }

                    // Send new ping
                    let nonce = rand::random::<u64>();
                    let ping = PingMessage { nonce };

                    ping_state.pending_nonce = Some(nonce);
                    ping_state.last_ping_sent = Instant::now();

                    drop(state); // Release lock

                    // Construct message manually since we can't easily access into_network_message helper if it's not pub
                    // Or use the one from protocol.rs if we implement Encode for PingMessage
                    // PeerManager::send_to_peer expects &NetworkMessage.
                    // We need to construct NetworkMessage.
                    let magic = peer_manager.magic();
                    let payload = crate::primitives::serialize::Encode::encode(&ping);
                    let msg = crate::network::message::NetworkMessage::new(magic, "ping", payload);

                    if let Err(e) = peer_manager.send_to_peer(peer_id, &msg) {
                        println!("Failed to send ping to peer {}: {}", peer_id, e);
                    }
                }
            }
        });
    }

    pub fn handle_pong(&self, peer_id: u64, nonce: u64) -> Result<(), String> {
        let mut state = self.ping_state.lock().unwrap();

        if let Some(ping_state) = state.get_mut(&peer_id) {
            // Verify nonce matches
            if ping_state.pending_nonce == Some(nonce) {
                ping_state.last_pong_received = Instant::now();
                ping_state.pending_nonce = None;
                ping_state.missed_pongs = 0;
                // Calculate latency here if needed: Instant::now() - ping_state.last_ping_sent
                Ok(())
            } else {
                Err("Pong nonce mismatch".to_string())
            }
        } else {
            // First pong or unexpected? Just ignore or log
            Ok(())
        }
    }
}
