use crate::network::addrman::AddressManager;
use crate::network::discovery::get_seed_nodes;
use crate::network::manager::PeerManager;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct RecoveryManager {
    peer_manager: Arc<PeerManager>,
    addr_manager: Arc<Mutex<AddressManager>>,
    state: Arc<Mutex<RecoveryState>>,
    network: crate::consensus::params::Network,
}

struct RecoveryState {
    last_block_time: Instant,
    last_peer_count: usize,
    partition_detected: bool,
    recovery_attempts: u32,
}

impl RecoveryManager {
    pub fn new(peer_manager: Arc<PeerManager>, addr_manager: Arc<Mutex<AddressManager>>, network: crate::consensus::params::Network) -> Self {
        Self {
            peer_manager,
            addr_manager,
            network,
            state: Arc::new(Mutex::new(RecoveryState {
                last_block_time: Instant::now(),
                last_peer_count: 0,
                partition_detected: false,
                recovery_attempts: 0,
            })),
        }
    }

    // Start recovery monitoring loop
    pub fn start(&self) {
        let manager = self.clone();

        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(60));

                // Check for partition
                if manager.check_partition() {
                    if let Err(e) = manager.recover() {
                        eprintln!("Recovery failed: {}", e);
                    }
                } else {
                    let peer_count = manager.peer_manager.active_peer_count();
                    if peer_count > 0 {
                        if let Ok(mut state) = manager.state.lock() {
                            if state.partition_detected {
                                println!("Network recovered - {} peers connected", peer_count);
                                state.partition_detected = false;
                                state.recovery_attempts = 0;
                            }
                        }
                    }
                }

                if let Ok(mut state) = manager.state.lock() {
                    state.last_peer_count = manager.peer_manager.active_peer_count();
                }
            }
        });
    }

    // Check if node is partitioned from network
    pub fn check_partition(&self) -> bool {
        let peer_count = self.peer_manager.active_peer_count();
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return false,
        };

        // Signs of partition:
        // 1. Zero active peers for >5 minutes
        if peer_count == 0 && state.last_block_time.elapsed() > Duration::from_secs(300) {
            return true;
        }

        // 2. No new blocks for >30 minutes (expected ~2.5 min)
        if state.last_block_time.elapsed() > Duration::from_secs(1800) {
            return true;
        }

        // 3. All peers suddenly disconnected
        if state.last_peer_count > 3 && peer_count == 0 {
            return true;
        }

        false
    }

    // Attempt network recovery
    pub fn recover(&self) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|e| format!("Poisoned mutex: {}", e))?;

        if !state.partition_detected {
            // First detection
            state.partition_detected = true;
            state.recovery_attempts = 0;
            println!("Network partition detected - initiating recovery");
        }

        state.recovery_attempts += 1;
        let attempt = state.recovery_attempts;
        drop(state);

        // Stage 1: Reconnect to known good peers
        if attempt == 1 {
            self.reconnect_to_known_peers()?;
        }
        // Stage 2: Try seed nodes
        else if attempt == 2 {
            self.reconnect_to_seeds()?;
        }
        // Stage 3: Aggressive reconnection
        else if attempt <= 5 {
            self.aggressive_reconnect()?;
        }
        // Stage 4: Full reset
        else {
            self.full_network_reset()?;
        }

        Ok(())
    }

    fn reconnect_to_known_peers(&self) -> Result<(), String> {
        println!("Stage 1: Reconnecting to known peers");

        // Get best addresses (previously successful)
        let addrs = self.addr_manager.lock().map_err(|e| format!("Poisoned mutex: {}", e))?.get_best_addresses(8);

        for addr in addrs {
            let _ = self.peer_manager.connect_to_peer(addr);
        }

        Ok(())
    }

    fn reconnect_to_seeds(&self) -> Result<(), String> {
        println!("Stage 2: Reconnecting to seed nodes");

        // We need to implement get_network on peer_manager or pass it in
        // For now, assume we can get it or hardcode regtest if missing
        // Actually, peer_manager doesn't expose network easily, but we can guess or store it
        // Let's assume Regtest for now or add get_network to PeerManager
        // For now, just use hardcoded seeds logic here or import from discovery
        let seeds = get_seed_nodes(self.network.clone());

        for seed in seeds {
            let _ = self.peer_manager.connect_to_peer(seed);
        }

        Ok(())
    }

    fn aggressive_reconnect(&self) -> Result<(), String> {
        let attempts = self.state.lock().map_err(|e| format!("Poisoned mutex: {}", e))?.recovery_attempts;
        println!(
            "Stage 3: Aggressive reconnection attempt {}",
            attempts
        );

        // Disconnect all peers
        self.force_reconnect();

        // Try many addresses
        let addrs = self.addr_manager.lock().map_err(|e| format!("Poisoned mutex: {}", e))?.get_random_addresses(20);

        for addr in addrs {
            let _ = self.peer_manager.connect_to_peer(addr);
        }

        Ok(())
    }

    fn full_network_reset(&self) -> Result<(), String> {
        println!("Stage 4: Full network reset");

        // Disconnect everything
        self.force_reconnect();

        let mut addr_mgr = self.addr_manager.lock().map_err(|e| format!("Poisoned mutex: {}", e))?;
        addr_mgr.clear();

        let seeds = get_seed_nodes(self.network.clone());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);

        for seed in &seeds {
            addr_mgr.add_address(*seed, 1, now);
        }
        drop(addr_mgr);

        // Reconnect to seeds
        for seed in seeds {
            let _ = self.peer_manager.connect_to_peer(seed);
        }

        Ok(())
    }

    pub fn force_reconnect(&self) {
        println!("Force reconnecting all peers");

        // Get all peer IDs
        let peer_ids = self.peer_manager.get_connected_peers();

        // Disconnect all
        for peer_id in peer_ids {
            let _ = self.peer_manager.disconnect_peer(peer_id);
        }
    }

    pub fn on_new_block(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.last_block_time = Instant::now();
        }
    }

    pub fn is_partitioned(&self) -> bool {
        self.state.lock().map(|s| s.partition_detected).unwrap_or(false)
    }

    pub fn get_attempts(&self) -> u32 {
        self.state.lock().map(|s| s.recovery_attempts).unwrap_or(0)
    }

    pub fn get_last_block_age_secs(&self) -> u64 {
        self.state
            .lock()
            .map(|s| s.last_block_time.elapsed().as_secs())
            .unwrap_or(0)
    }
}

// Clone implementation for RecoveryManager
impl Clone for RecoveryManager {
    fn clone(&self) -> Self {
        Self {
            peer_manager: Arc::clone(&self.peer_manager),
            addr_manager: Arc::clone(&self.addr_manager),
            state: Arc::clone(&self.state),
            network: self.network.clone(),
        }
    }
}
