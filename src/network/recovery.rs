use crate::consensus::chain::ChainState;
use crate::network::addrman::AddressManager;
use crate::network::manager::PeerManager;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct RecoveryManager {
    peer_manager: Arc<PeerManager>,
    addr_manager: Arc<Mutex<AddressManager>>,
    state: Arc<Mutex<RecoveryState>>,
    chain: Arc<RwLock<ChainState>>,
    /// Seed nodes configured via --seed-nodes CLI flag.  Used by all recovery
    /// stages so that Stage 4 full_network_reset() can always reconnect even
    /// when get_seed_nodes() returns an empty list for testnet.
    configured_seeds: Vec<SocketAddr>,
}

struct RecoveryState {
    last_peer_count: usize,
    partition_detected: bool,
    recovery_attempts: u32,
    last_recover_call: Instant,
}

impl RecoveryManager {
    pub fn new(
        peer_manager: Arc<PeerManager>,
        addr_manager: Arc<Mutex<AddressManager>>,
        network: crate::consensus::params::Network,
        chain: Arc<RwLock<ChainState>>,
        configured_seeds: Vec<SocketAddr>,
    ) -> Self {
        // Pre-populate addr_manager with configured seeds so Stage 1
        // (reconnect_to_known_peers via get_best_addresses) can find them.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        {
            let mut am = addr_manager.lock().unwrap();
            for seed in &configured_seeds {
                am.add_address(*seed, 1, now);
            }
        }
        let _ = network; // network field removed; seeds are explicit now
        Self {
            peer_manager,
            addr_manager,
            chain,
            configured_seeds,
            state: Arc::new(Mutex::new(RecoveryState {
                last_peer_count: 0,
                partition_detected: false,
                recovery_attempts: 0,
                last_recover_call: Instant::now()
                    .checked_sub(Duration::from_secs(60))
                    .unwrap_or_else(Instant::now),
            })),
        }
    }

    // Returns how many seconds ago the chain tip block was produced.
    // Uses try_read() so it never blocks the caller.  Returns 0 on lock
    // contention (treat as "recently updated" — don't falsely trigger).
    fn tip_age_secs(&self) -> u64 {
        if let Ok(chain) = self.chain.try_read() {
            if let Ok(Some(tip)) = chain.get_tip() {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                return now.saturating_sub(tip.block.header.timestamp);
            }
        }
        0
    }

    // Start recovery monitoring loop
    pub fn start(&self) {
        let manager = self.clone();

        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(30));

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

    // Check if node is partitioned from network.
    // All conditions use the chain tip timestamp — the ground truth for when
    // the chain last advanced — rather than an on_new_block() callback, which
    // is not called by all mining paths (e.g. mineblocks RPC) and which
    // would falsely age a solo-mining node that has no peers.
    pub fn check_partition(&self) -> bool {
        let peer_count = self.peer_manager.active_peer_count();
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return false,
        };

        let age = self.tip_age_secs();

        // 1. Zero active peers and chain tip is stale (>30 s old).
        //    At 150 s block time a healthy solo miner keeps tip_age <150 s,
        //    so this only fires when mining has also stopped.
        if peer_count == 0 && age > 30 {
            return true;
        }

        // 2. Chain tip has not advanced for >30 minutes regardless of peers.
        if age > 1800 {
            return true;
        }

        // 3. All peers suddenly disconnected (was previously >3, now 0).
        if state.last_peer_count > 3 && peer_count == 0 {
            return true;
        }

        false
    }

    // Attempt network recovery — rate-limited to once per 30s to prevent thrash.
    pub fn recover(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| format!("Poisoned mutex: {}", e))?;

        if state.last_recover_call.elapsed() < Duration::from_secs(30) {
            log::debug!(
                "recover() skipped — rate-limited (last call {:.1}s ago)",
                state.last_recover_call.elapsed().as_secs_f64()
            );
            return Ok(());
        }
        state.last_recover_call = Instant::now();

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
        let addrs = self
            .addr_manager
            .lock()
            .map_err(|e| format!("Poisoned mutex: {}", e))?
            .get_best_addresses(8);

        for addr in addrs {
            let _ = self.peer_manager.connect_to_peer(addr);
        }

        Ok(())
    }

    fn reconnect_to_seeds(&self) -> Result<(), String> {
        println!("Stage 2: Reconnecting to seed nodes");

        for seed in &self.configured_seeds {
            let _ = self.peer_manager.connect_to_peer(*seed);
        }

        Ok(())
    }

    fn aggressive_reconnect(&self) -> Result<(), String> {
        let attempts = self
            .state
            .lock()
            .map_err(|e| format!("Poisoned mutex: {}", e))?
            .recovery_attempts;
        println!("Stage 3: Aggressive reconnection attempt {}", attempts);

        // Disconnect all peers
        self.force_reconnect();

        // Try many addresses
        let addrs = self
            .addr_manager
            .lock()
            .map_err(|e| format!("Poisoned mutex: {}", e))?
            .get_random_addresses(20);

        for addr in addrs {
            let _ = self.peer_manager.connect_to_peer(addr);
        }

        Ok(())
    }

    fn full_network_reset(&self) -> Result<(), String> {
        println!("Stage 4: Full network reset");

        // Disconnect everything
        self.force_reconnect();

        // Reset addr_manager to only our configured seeds.
        {
            let mut addr_mgr = self
                .addr_manager
                .lock()
                .map_err(|e| format!("Poisoned mutex: {}", e))?;
            addr_mgr.clear();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as u32)
                .unwrap_or(0);
            for seed in &self.configured_seeds {
                addr_mgr.add_address(*seed, 1, now);
            }
        }

        // Clear any DoS failed-attempt cooldowns for configured seeds so a
        // transient TCP failure from an earlier recovery stage does not block
        // this reconnection attempt.
        for seed in &self.configured_seeds {
            self.peer_manager.clear_failed_attempt(seed.ip());
        }

        // Reconnect to configured seeds.
        for seed in &self.configured_seeds {
            let _ = self.peer_manager.connect_to_peer(*seed);
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

    /// No-op — retained for API compatibility.  last_block_age is now derived
    /// from the chain tip timestamp directly; callers that previously notified
    /// the recovery manager of new blocks no longer need to do so.
    pub fn on_new_block(&self) {}

    pub fn is_partitioned(&self) -> bool {
        self.state
            .lock()
            .map(|s| s.partition_detected)
            .unwrap_or(false)
    }

    pub fn get_attempts(&self) -> u32 {
        self.state.lock().map(|s| s.recovery_attempts).unwrap_or(0)
    }

    /// Returns the age of the chain tip in seconds (ground truth for chain staleness).
    pub fn get_last_block_age_secs(&self) -> u64 {
        self.tip_age_secs()
    }
}

// Clone implementation for RecoveryManager
impl Clone for RecoveryManager {
    fn clone(&self) -> Self {
        Self {
            peer_manager: Arc::clone(&self.peer_manager),
            addr_manager: Arc::clone(&self.addr_manager),
            state: Arc::clone(&self.state),
            chain: Arc::clone(&self.chain),
            configured_seeds: self.configured_seeds.clone(),
        }
    }
}
