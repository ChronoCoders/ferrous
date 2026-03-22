use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::network::batch::{BroadcastCache, MessageBatcher};
use crate::network::connection::PeerConnection;
use crate::network::discovery::PeerDiscovery;
use crate::network::dos::DosProtection;
use crate::network::handshake::{perform_handshake, perform_inbound_handshake};
use crate::network::keepalive::KeepaliveManager;
use crate::network::listener::NetworkListener;
use crate::network::message::NetworkMessage;
use crate::network::peer::{Peer, PeerState};
use crate::network::protocol::{InvVector, MessagePayload, PongMessage};
use crate::network::recovery::RecoveryManager;
use crate::network::relay::BlockRelay;
use crate::network::security::NetworkSecurity;
use crate::network::stats::NetworkStats;
use crate::network::sync::SyncManager;
use crate::network::validation::Validate;
use crate::primitives::serialize::Encode;

#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub id: u64,
    pub addr: SocketAddr,
    pub state: PeerState,
    pub version: Option<u32>,
    pub services: u64,
    pub start_height: u32,
    pub inbound: bool,
}

pub struct PeerManager {
    peers: Arc<Mutex<HashMap<u64, Peer>>>,
    next_peer_id: Arc<Mutex<u64>>,
    magic: [u8; 4],
    max_peers: usize,
    our_version: u32,
    our_services: u64,
    our_height: u32,
    relay: Arc<Mutex<Option<Arc<BlockRelay>>>>,
    sync_manager: Arc<Mutex<Option<Arc<SyncManager>>>>,
    discovery: Arc<Mutex<Option<Arc<PeerDiscovery>>>>,
    keepalive: Arc<Mutex<Option<Arc<KeepaliveManager>>>>,
    stats: Arc<Mutex<Option<Arc<NetworkStats>>>>,
    recovery: Arc<Mutex<Option<Arc<RecoveryManager>>>>,
    dos_protection: Arc<Mutex<DosProtection>>,
    batcher: Arc<Mutex<MessageBatcher>>,
    broadcast_cache: Arc<Mutex<BroadcastCache>>,
    security: Arc<Mutex<NetworkSecurity>>,
}

impl PeerManager {
    pub fn new(
        magic: [u8; 4],
        max_peers: usize,
        version: u32,
        services: u64,
        start_height: u32,
    ) -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
            next_peer_id: Arc::new(Mutex::new(0)),
            magic,
            max_peers,
            our_version: version,
            our_services: services,
            our_height: start_height,
            relay: Arc::new(Mutex::new(None)),
            sync_manager: Arc::new(Mutex::new(None)),
            discovery: Arc::new(Mutex::new(None)),
            keepalive: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(None)),
            recovery: Arc::new(Mutex::new(None)),
            dos_protection: Arc::new(Mutex::new(DosProtection::new())),
            batcher: Arc::new(Mutex::new(MessageBatcher::new(magic))),
            broadcast_cache: Arc::new(Mutex::new(BroadcastCache::new(1000))),
            security: Arc::new(Mutex::new(NetworkSecurity::new())),
        }
    }

    pub fn set_recovery(&self, recovery: Arc<RecoveryManager>) {
        let mut guard = self.recovery.lock().unwrap();
        *guard = Some(recovery);
    }

    pub fn set_stats(&self, stats: Arc<NetworkStats>) {
        let mut guard = self.stats.lock().unwrap();
        *guard = Some(stats);
    }

    pub fn get_peers(&self) -> Arc<Mutex<HashMap<u64, Peer>>> {
        Arc::clone(&self.peers)
    }

    pub fn set_relay(&self, relay: Arc<BlockRelay>) {
        let mut guard = self.relay.lock().unwrap();
        *guard = Some(relay);
    }

    pub fn set_sync_manager(&self, sync: Arc<SyncManager>) {
        let mut guard = self.sync_manager.lock().unwrap();
        *guard = Some(sync);
    }

    pub fn set_discovery(&self, discovery: Arc<PeerDiscovery>) {
        let mut guard = self.discovery.lock().unwrap();
        *guard = Some(discovery);
    }

    pub fn set_keepalive(&self, keepalive: Arc<KeepaliveManager>) {
        let mut guard = self.keepalive.lock().unwrap();
        *guard = Some(keepalive);
    }

    pub fn get_connected_peers(&self) -> Vec<u64> {
        self.peers.lock().unwrap().keys().cloned().collect()
    }

    pub fn send_to_peer(&self, peer_id: u64, message: &NetworkMessage) -> Result<(), String> {
        let mut peers = self.peers.lock().unwrap();
        if let Some(peer) = peers.get_mut(&peer_id) {
            // Allow sending even if connecting (e.g. handshake messages)
            peer.send(message)
        } else {
            Err("Peer not found".to_string())
        }
    }

    pub fn disconnect_peer(&self, peer_id: u64) -> Result<(), String> {
        let info = {
            let mut peers = self.peers.lock().unwrap();
            peers.remove(&peer_id).map(|p| (p.addr.ip(), p.inbound))
        };
        // peers lock released.
        if let Some((ip, inbound)) = info {
            self.dos_protection
                .lock()
                .unwrap()
                .record_disconnection(peer_id, ip, inbound);
            self.batcher.lock().unwrap().clear_peer(peer_id);
            self.broadcast_cache.lock().unwrap().clear_peer(peer_id);
            self.security.lock().unwrap().remove_peer(peer_id);
            Ok(())
        } else {
            Err("Peer not found".to_string())
        }
    }

    pub fn check_network_health(&self) {
        let security = self.security.lock().unwrap();
        if security.detect_eclipse_attempt() {
            println!("Potential Eclipse attack detected!");
            // Log stats
            let (netgroups, peers, max_pct) = security.get_diversity_stats();
            println!(
                "Network diversity: {} netgroups, {} peers, max {:.2}%",
                netgroups,
                peers,
                max_pct * 100.0
            );
        }
    }

    pub fn punish_peer(&self, peer_id: u64, score: u32) {
        let should_remove = {
            let mut peers = self.peers.lock().unwrap();
            if let Some(peer) = peers.get_mut(&peer_id) {
                peer.add_ban_score(score);
                if peer.should_ban() {
                    println!("Peer {} banned (score: {})", peer_id, peer.get_ban_score());
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        // peers lock released before removing/cleaning up.
        if should_remove {
            let _ = self.disconnect_peer(peer_id);
        }
    }

    pub fn cleanup_inactive_peers(&self) {
        let timeout = Duration::from_secs(20 * 60); // 20 minutes
        let mut to_disconnect: Vec<u64> = Vec::new();

        {
            let peers = self.peers.lock().unwrap();
            for (peer_id, peer) in peers.iter() {
                if peer.time_since_last_recv() > timeout {
                    to_disconnect.push(*peer_id);
                }
            }
        }

        for peer_id in to_disconnect {
            println!("Disconnecting inactive peer {}", peer_id);
            let _ = self.disconnect_peer(peer_id);
        }
    }

    pub fn start_maintenance(&self) {
        let peers = Arc::clone(&self.peers);

        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(60));

                // Cleanup logic inline to avoid cloning whole manager structure just for this
                let timeout = Duration::from_secs(20 * 60);

                {
                    let mut peers_guard = peers.lock().unwrap();
                    let keys: Vec<u64> = peers_guard.keys().cloned().collect();

                    for peer_id in keys {
                        let should_remove = if let Some(peer) = peers_guard.get(&peer_id) {
                            if peer.time_since_last_recv() > timeout {
                                true
                            } else {
                                let recv_rate = peer.get_recv_rate();
                                if recv_rate > 10_000_000.0 {
                                    println!(
                                        "Peer {} exceeded bandwidth limit: {:.2} MB/s",
                                        peer_id,
                                        recv_rate / 1_000_000.0
                                    );
                                    true
                                } else {
                                    false
                                }
                            }
                        } else {
                            false
                        };

                        if should_remove {
                            println!("Disconnecting peer {}", peer_id);
                            peers_guard.remove(&peer_id);
                        }
                    }
                }
            }
        });
    }

    pub fn connect_to_peer(&self, addr: SocketAddr) -> Result<u64, String> {
        let ip = addr.ip();

        // DoS check
        let dos = self.dos_protection.lock().unwrap();
        if !dos.can_connect_outbound(ip) {
            return Err("Connection rejected by DoS protection".to_string());
        }
        drop(dos);

        let peers = self.peers.lock().unwrap();
        if peers.len() >= self.max_peers {
            return Err("Max peers reached".to_string());
        }
        drop(peers);

        // Skip if already connected to this IP
        let peers = self.peers.lock().unwrap();
        let already_connected = peers.values().any(|p| p.addr.ip() == ip);
        drop(peers);
        if already_connected {
            return Err(format!("Already connected to {}", ip));
        }

        let stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(s) => s,
            Err(e) => {
                let mut dos = self.dos_protection.lock().unwrap();
                dos.record_failed_attempt(ip);
                return Err(format!("Failed to connect to {}: {}", addr, e));
            }
        };

        let conn = PeerConnection::new(stream, self.magic)?;

        let mut id_guard = self.next_peer_id.lock().unwrap();
        let id = *id_guard;
        *id_guard += 1;
        drop(id_guard);

        let peer = Peer::new_outbound(id, conn);

        let peers_clone = Arc::clone(&self.peers);
        let dos_protection_clone = Arc::clone(&self.dos_protection);
        let security_clone = Arc::clone(&self.security);
        let sync_manager_clone = Arc::clone(&self.sync_manager);
        let our_version = self.our_version;
        let our_services = self.our_services;

        // Extract Arc clone then release the wrapper lock before calling
        // get_local_height(), which acquires chain.read() internally.
        // Holding the wrapper lock during chain.read() would block any
        // concurrent inbound handshake thread that also needs the wrapper lock.
        let our_height = {
            let sync_opt = {
                let g = self.sync_manager.lock().unwrap();
                g.as_ref().cloned()
            };
            sync_opt
                .map(|s| s.get_local_height())
                .unwrap_or(self.our_height)
        };

        let mut peers = peers_clone.lock().unwrap();
        peers.insert(id, peer);
        drop(peers);

        thread::spawn(move || {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);

            // Re-acquire to mutate for handshake
            let mut peers = peers_clone.lock().unwrap();
            if let Some(mut peer) = peers.remove(&id) {
                drop(peers);
                println!("Starting handshake with outbound peer {}", addr);
                match perform_handshake(&mut peer, our_version, our_services, our_height, nonce) {
                    Ok(_) => {
                        println!("Handshake success with {}", addr);
                        {
                            let mut peers = peers_clone.lock().unwrap();
                            peer.connected_at = Instant::now();
                            peer.last_recv = Instant::now();
                            peer.state = PeerState::Active;
                            peers.insert(id, peer);
                        } // peers lock released before start_sync

                        // Record successful connection
                        let mut dos = dos_protection_clone.lock().unwrap();
                        dos.record_connection(id, ip, false);

                        let mut security = security_clone.lock().unwrap();
                        security.record_peer(id, ip);

                        // Trigger sync — clone Arc out from wrapper lock before calling
                        // start_sync, which may block on send_to_peer (30s write timeout).
                        // Holding the wrapper lock during start_sync would block any
                        // concurrent inbound handshake thread waiting for the same lock.
                        let sync_opt = {
                            let g = sync_manager_clone.lock().unwrap();
                            g.as_ref().cloned()
                        };
                        if let Some(sync) = sync_opt {
                            let _ = sync.start_sync(id);
                        }
                    }
                    Err(e) => {
                        println!("Handshake failed with {}: {}", addr, e);
                        // Cleanup on failure
                        let mut peers = peers_clone.lock().unwrap();
                        if let Some(peer) = peers.remove(&id) {
                            let ip = peer.addr.ip();
                            let inbound = peer.inbound;
                            // Record disconnection for proper DoS accounting
                            let mut dos = dos_protection_clone.lock().unwrap();
                            dos.record_disconnection(id, ip, inbound);
                        }
                    }
                }
            }
        });

        Ok(id)
    }

    pub fn get_peer_count(&self) -> usize {
        let peers = self.peers.lock().unwrap();
        // Count only peers that have completed handshake (Active)
        peers
            .values()
            .filter(|p| p.state == PeerState::Active)
            .count()
    }

    pub fn active_peer_count(&self) -> usize {
        self.peers
            .lock()
            .unwrap()
            .values()
            .filter(|p| p.state == PeerState::Active)
            .count()
    }

    pub fn get_peer_addrs(&self) -> Vec<SocketAddr> {
        self.peers
            .lock()
            .unwrap()
            .values()
            .map(|p| p.addr)
            .collect()
    }

    pub fn outbound_peer_count(&self) -> usize {
        let peers = self.peers.lock().unwrap();
        peers
            .values()
            .filter(|p| !p.inbound && p.state == PeerState::Active)
            .count()
    }

    pub fn magic(&self) -> [u8; 4] {
        self.magic
    }

    pub fn sync_manager(&self) -> Arc<Mutex<Option<Arc<SyncManager>>>> {
        Arc::clone(&self.sync_manager)
    }

    pub fn get_peer_start_height(&self, peer_id: u64) -> Option<u32> {
        let peers = self.peers.lock().unwrap();
        peers.get(&peer_id).map(|p| p.start_height)
    }

    pub fn update_peer_height(&self, peer_id: u64, height: u32) {
        let mut peers = self.peers.lock().unwrap();
        if let Some(peer) = peers.get_mut(&peer_id) {
            if height > peer.start_height {
                peer.start_height = height;
            }
        }
    }

    pub fn start_listener(&self, addr: SocketAddr) -> Result<(), String> {
        let listener = NetworkListener::new(addr, self.magic, self.max_peers);

        let peers_clone = Arc::clone(&self.peers);
        let next_peer_id_clone = Arc::clone(&self.next_peer_id);
        let dos_protection_clone = Arc::clone(&self.dos_protection);
        let security_clone = Arc::clone(&self.security);
        let sync_manager_clone = Arc::clone(&self.sync_manager);
        let max_peers = self.max_peers;
        let magic = self.magic;

        listener.start(move |conn| {
            let peer_addr = conn.peer_addr();
            let ip = peer_addr.ip();

            // DoS check
            let mut dos = dos_protection_clone.lock().unwrap();
            let is_trusted = ip.to_string() == "45.77.153.141" || ip.to_string() == "45.77.64.221";

            if !is_trusted && !dos.can_accept_inbound(ip) {
                log::warn!("inbound: rejected {} (DoS protection)", ip);
                dos.record_failed_attempt(ip);
                return;
            }
            drop(dos);

            // Diversity check
            let is_regtest = magic == [0xfa, 0xbf, 0xb5, 0xda];
            let security = security_clone.lock().unwrap();
            let total_peers = {
                let peers = peers_clone.lock().unwrap();
                peers.len()
            };
            if !is_regtest && !is_trusted && !security.can_accept_for_diversity(ip, total_peers) {
                log::warn!(
                    "inbound: rejected {} (diversity, total_peers={})",
                    ip,
                    total_peers
                );
                return;
            }
            drop(security);

            // Skip if already connected to this IP (but allow trusted peers to reconnect)
            let is_trusted = ip.to_string() == "45.77.153.141" || ip.to_string() == "45.77.64.221";
            if !is_trusted {
                let already_connected = peers_clone
                    .lock()
                    .unwrap()
                    .values()
                    .any(|p| p.addr.ip() == ip);
                if already_connected {
                    log::warn!("inbound: rejected {} (duplicate — already connected)", ip);
                    return;
                }
            }

            let peer_count = peers_clone.lock().unwrap().len();
            if peer_count >= max_peers {
                log::warn!(
                    "inbound: rejected {} (max_peers={} reached, current={})",
                    peer_addr,
                    max_peers,
                    peer_count
                );
                return;
            }

            println!("New inbound connection from {}", peer_addr);

            let mut next_id = next_peer_id_clone.lock().unwrap();
            let id = *next_id;
            *next_id += 1;
            drop(next_id);

            let mut peer = Peer::new_inbound(id, conn);

            // Let's spawn a handshake thread for inbound too
            let peers_inner = Arc::clone(&peers_clone);
            let dos_protection_inner = Arc::clone(&dos_protection_clone);
            let security_inner = Arc::clone(&security_clone);
            let sync_manager_inner = Arc::clone(&sync_manager_clone);

            thread::spawn(move || {
                log::debug!("inbound [{}]: thread started", ip);
                let version = 70015;
                let services = 0;
                // Use the non-blocking variant: the height in VERSION is
                // advisory and 0 is a safe fallback.  get_local_height()
                // calls chain.read() which can block for tens of seconds
                // under write pressure (burst add_block calls), causing the
                // remote's 30 s outbound timeout to expire before the
                // handshake even starts.
                let height = {
                    let sync_opt = {
                        let g = sync_manager_inner.lock().unwrap();
                        g.as_ref().cloned()
                    };
                    sync_opt.map(|s| s.try_get_local_height()).unwrap_or(0)
                };
                log::debug!(
                    "inbound [{}]: height={}, calling perform_inbound_handshake",
                    ip,
                    height
                );
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                match perform_inbound_handshake(&mut peer, version, services, height, nonce) {
                    Ok(_) => {
                        println!("Inbound handshake success from {}", ip);
                        peer.state = PeerState::Active;
                        peer.connected_at = Instant::now();
                        peer.last_recv = Instant::now();
                        peers_inner.lock().unwrap().insert(id, peer);

                        let mut dos = dos_protection_inner.lock().unwrap();
                        dos.record_connection(id, ip, true);

                        let mut security = security_inner.lock().unwrap();
                        security.record_peer(id, ip);

                        // Trigger sync — clone Arc out from wrapper lock before calling
                        // start_sync (same reason as outbound path above).
                        let sync_opt = {
                            let g = sync_manager_inner.lock().unwrap();
                            g.as_ref().cloned()
                        };
                        if let Some(sync) = sync_opt {
                            let _ = sync.start_sync(id);
                        }
                    }
                    Err(e) => {
                        log::debug!("inbound [{}]: handshake error — {}", ip, e);
                        println!("Inbound handshake failed from {}: {}", ip, e);
                    }
                }
            });
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_message(
        id: u64,
        payload: &MessagePayload,
        magic: [u8; 4],
        peers: &Arc<Mutex<HashMap<u64, Peer>>>,
        relay: &Arc<Mutex<Option<Arc<BlockRelay>>>>,
        sync: &Arc<Mutex<Option<Arc<SyncManager>>>>,
        discovery: &Arc<Mutex<Option<Arc<PeerDiscovery>>>>,
        keepalive: &Arc<Mutex<Option<Arc<KeepaliveManager>>>>,
        stats: &Arc<Mutex<Option<Arc<NetworkStats>>>>,
        recovery: &Arc<Mutex<Option<Arc<RecoveryManager>>>>,
    ) {
        // Handle Ping/Pong directly here
        match payload {
            MessagePayload::Ping(ping) => {
                // Auto-respond with pong
                let pong = PongMessage { nonce: ping.nonce };
                let encoded = Encode::encode(&pong);
                let resp = NetworkMessage::new(magic, "pong", encoded);
                let resp_size = resp.encoded_size();

                let mut peers_guard = peers.lock().unwrap();
                if let Some(peer) = peers_guard.get_mut(&id) {
                    let _ = peer.send(&resp);
                    peer.record_sent(resp_size);
                }
                // Record sent stats
                {
                    let stats_guard = stats.lock().unwrap();
                    if let Some(stats) = &*stats_guard {
                        stats.record_message_sent(resp_size);
                    }
                }
            }
            MessagePayload::Pong(pong) => {
                let keepalive_guard = keepalive.lock().unwrap();
                if let Some(keepalive) = &*keepalive_guard {
                    let _ = keepalive.handle_pong(id, pong.nonce);
                }
            }
            _ => {}
        }

        // Dispatch to Relay
        {
            let relay_guard = relay.lock().unwrap();
            if let Some(relay) = &*relay_guard {
                match payload {
                    MessagePayload::Inv(inv) => {
                        let _ = relay.handle_inv(id, inv);
                    }
                    MessagePayload::GetData(getdata) => {
                        if let Err(e) = relay.handle_getdata(id, getdata) {
                            log::warn!("dispatch: handle_getdata error from peer {}: {}", id, e);
                        }
                    }
                    MessagePayload::Block(block) => {
                        log::debug!("dispatch: received block message from peer {}", id);
                        if let Err(e) = relay.handle_block(id, block) {
                            log::warn!("dispatch: handle_block error from peer {}: {}", id, e);
                        }
                        // Record block received
                        let stats_guard = stats.lock().unwrap();
                        if let Some(stats) = &*stats_guard {
                            stats.record_block_received();
                        }
                        // Notify recovery manager
                        let recovery_guard = recovery.lock().unwrap();
                        if let Some(recovery) = &*recovery_guard {
                            recovery.on_new_block();
                        }
                    }
                    MessagePayload::Tx(tx) => {
                        let _ = relay.handle_transaction(id, &tx.transaction);
                        // Record tx received
                        let stats_guard = stats.lock().unwrap();
                        if let Some(stats) = &*stats_guard {
                            stats.record_transaction_received();
                        }
                    }
                    _ => {}
                }
            }
        }

        // Dispatch to Sync
        {
            let sync_guard = sync.lock().unwrap();
            if let Some(sync) = &*sync_guard {
                match payload {
                    MessagePayload::Headers(headers) => {
                        if let Err(e) = sync.handle_headers(id, headers) {
                            log::warn!("SyncManager: handle_headers error from peer {}: {}", id, e);
                        }
                    }
                    MessagePayload::GetHeaders(getheaders) => {
                        if let Err(e) = sync.handle_getheaders(id, getheaders) {
                            log::warn!(
                                "SyncManager: handle_getheaders error from peer {}: {}",
                                id,
                                e
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        // Dispatch to Discovery
        {
            let discovery_guard = discovery.lock().unwrap();
            if let Some(discovery) = &*discovery_guard {
                match payload {
                    MessagePayload::Addr(addr) => {
                        let _ = discovery.handle_addr(addr);
                    }
                    MessagePayload::GetAddr(_) => {
                        let _ = discovery.handle_getaddr(id);
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn start_batch_flusher(&self) {
        let batcher = Arc::clone(&self.batcher);
        let peers = Arc::clone(&self.peers);

        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(100));

            let mut batcher = batcher.lock().unwrap();
            let batches = batcher.flush_all_needed();
            drop(batcher);

            if !batches.is_empty() {
                let mut peers = peers.lock().unwrap();
                for (peer_id, messages) in batches {
                    if let Some(peer) = peers.get_mut(&peer_id) {
                        for msg in messages {
                            if let Err(e) = peer.send(&msg) {
                                println!("Failed to send batched message: {}", e);
                            }
                        }
                    }
                }
            }
        });
    }

    pub fn start_message_handler(&self) {
        self.start_batch_flusher();

        let peers_clone = Arc::clone(&self.peers);
        let relay_clone = Arc::clone(&self.relay);
        let sync_clone = Arc::clone(&self.sync_manager);
        let discovery_clone = Arc::clone(&self.discovery);
        let keepalive_clone: Arc<Mutex<Option<Arc<KeepaliveManager>>>> =
            Arc::clone(&self.keepalive);
        let stats_clone: Arc<Mutex<Option<Arc<NetworkStats>>>> = Arc::clone(&self.stats);
        let recovery_clone: Arc<Mutex<Option<Arc<RecoveryManager>>>> = Arc::clone(&self.recovery);
        let dos_protection_clone = Arc::clone(&self.dos_protection);
        let batcher_clone = Arc::clone(&self.batcher);
        let broadcast_cache_clone = Arc::clone(&self.broadcast_cache);
        let security_clone = Arc::clone(&self.security);

        // Capture magic for auto-pong (can't use self inside thread)
        let magic = self.magic;

        thread::spawn(move || {
            let (block_tx, block_rx) = mpsc::sync_channel::<(u64, NetworkMessage)>(1024);

            {
                let relay = Arc::clone(&relay_clone);
                std::thread::Builder::new()
                    .name("block-dispatch-worker".into())
                    .spawn(move || {
                        while let Ok((peer_id, msg)) = block_rx.recv() {
                            let payload = match msg.parse_payload() {
                                Ok(p) => p,
                                Err(e) => {
                                    log::warn!(
                                        "block-worker: failed to parse block message from peer {}: {:?}",
                                        peer_id, e
                                    );
                                    continue;
                                }
                            };

                            if let Err(e) = payload.validate() {
                                log::warn!(
                                    "block-worker: invalid block payload from peer {}: {:?}",
                                    peer_id, e
                                );
                                continue;
                            }

                            let MessagePayload::Block(block) = payload else {
                                continue;
                            };

                            let relay_guard = relay.lock().unwrap();
                            if let Some(relay) = &*relay_guard {
                                if let Err(e) = relay.handle_block(peer_id, &block) {
                                    log::warn!(
                                        "block-worker: handle_block error from peer {}: {}",
                                        peer_id, e
                                    );
                                }
                            }
                        }
                    })
                    .expect("failed to spawn block-dispatch-worker");
            }

            loop {
                // Collect IDs first to avoid holding lock while processing
                let mut peer_ids: Vec<u64> = Vec::new();
                {
                    let peers = peers_clone.lock().unwrap();
                    for (id, peer) in peers.iter() {
                        if peer.state == PeerState::Active {
                            peer_ids.push(*id);
                        }
                    }
                }

                'peer_loop: for id in peer_ids {
                    // Lock peers to access peer
                    let mut peers = peers_clone.lock().unwrap();
                    if let Some(peer) = peers.get_mut(&id) {
                        // Try to read message
                        match peer.receive() {
                            Ok(Some(msg)) => {
                                // VALIDATION CHECKPOINT 1: Message structure and size
                                if let Err(e) = msg.validate() {
                                    println!("Invalid message from {}: {:?}", id, e);
                                    peer.add_ban_score(10);
                                    if peer.should_ban() {
                                        // NLL: peer's last use is peer.should_ban() above.
                                        let dci =
                                            peers.remove(&id).map(|p| (p.addr.ip(), p.inbound));
                                        let rem = peers.len();
                                        drop(peers);
                                        {
                                            let sg = sync_clone.lock().unwrap();
                                            if let Some(sync) = &*sg {
                                                sync.on_peer_disconnected(id);
                                            }
                                        }
                                        batcher_clone.lock().unwrap().clear_peer(id);
                                        broadcast_cache_clone.lock().unwrap().clear_peer(id);
                                        if let Some((ip, inbound)) = dci {
                                            dos_protection_clone
                                                .lock()
                                                .unwrap()
                                                .record_disconnection(id, ip, inbound);
                                            security_clone.lock().unwrap().remove_peer(id);
                                        }
                                        if rem == 0 {
                                            let recovery_bg = Arc::clone(&recovery_clone);
                                            thread::spawn(move || {
                                                let rg = recovery_bg.lock().unwrap();
                                                if let Some(r) = &*rg {
                                                    let _ = r.recover();
                                                }
                                            });
                                        }
                                        continue 'peer_loop;
                                    }
                                } else {
                                    // Only process if valid
                                    let msg_size = msg.encoded_size();
                                    // Update last_recv and record bandwidth
                                    peer.update_last_recv();
                                    peer.record_received(msg_size);

                                    // Record stats
                                    {
                                        let stats_guard = stats_clone.lock().unwrap();
                                        if let Some(stats) = &*stats_guard {
                                            stats.record_message_received(msg_size);
                                        }
                                    }

                                    // Check general message rate
                                    if !peer.check_message_rate() {
                                        println!("Peer {} exceeded message rate limit", id);
                                        peer.add_ban_score(10);
                                        if peer.should_ban() {
                                            println!("Banning peer {} for rate limit abuse", id);
                                            {
                                                let stats_guard = stats_clone.lock().unwrap();
                                                if let Some(stats) = &*stats_guard {
                                                    stats.record_banned_peer();
                                                }
                                            }
                                            // NLL: peer's last use is peer.should_ban() above.
                                            let dci =
                                                peers.remove(&id).map(|p| (p.addr.ip(), p.inbound));
                                            let rem = peers.len();
                                            drop(peers);
                                            {
                                                let sg = sync_clone.lock().unwrap();
                                                if let Some(sync) = &*sg {
                                                    sync.on_peer_disconnected(id);
                                                }
                                            }
                                            batcher_clone.lock().unwrap().clear_peer(id);
                                            broadcast_cache_clone.lock().unwrap().clear_peer(id);
                                            if let Some((ip, inbound)) = dci {
                                                dos_protection_clone
                                                    .lock()
                                                    .unwrap()
                                                    .record_disconnection(id, ip, inbound);
                                                security_clone.lock().unwrap().remove_peer(id);
                                            }
                                            if rem == 0 {
                                                let recovery_bg = Arc::clone(&recovery_clone);
                                                thread::spawn(move || {
                                                    let rg = recovery_bg.lock().unwrap();
                                                    if let Some(r) = &*rg {
                                                        let _ = r.recover();
                                                    }
                                                });
                                            }
                                            continue 'peer_loop;
                                        } else {
                                            // Record rate limit event
                                            {
                                                let stats_guard = stats_clone.lock().unwrap();
                                                if let Some(stats) = &*stats_guard {
                                                    stats.record_rate_limited();
                                                }
                                            }
                                        }
                                    }

                                    // Process valid payload
                                    {
                                        drop(peers); // Drop lock before handling

                                        let command = msg.command_string();
                                        if command == "block" {
                                            log::debug!(
                                                "dispatch: queueing block message from peer {}, payload_len={}",
                                                id,
                                                msg.payload.len()
                                            );
                                            if let Err(e) = block_tx.send((id, msg)) {
                                                log::warn!(
                                                    "dispatch: failed to enqueue block message from peer {}: {}",
                                                    id, e
                                                );
                                            } else {
                                                let stats_guard = stats_clone.lock().unwrap();
                                                if let Some(stats) = &*stats_guard {
                                                    stats.record_block_received();
                                                }
                                                let recovery_guard = recovery_clone.lock().unwrap();
                                                if let Some(recovery) = &*recovery_guard {
                                                    recovery.on_new_block();
                                                }
                                            }
                                            continue 'peer_loop;
                                        }

                                        match msg.parse_payload() {
                                            Ok(payload) => {
                                                // VALIDATION CHECKPOINT 2: Payload content
                                                if let Err(e) = payload.validate() {
                                                    println!(
                                                        "Invalid payload from {}: {:?}",
                                                        id, e
                                                    );
                                                    // Severe violation
                                                    // Re-acquire lock to punish
                                                    let mut peers = peers_clone.lock().unwrap();
                                                    if let Some(peer) = peers.get_mut(&id) {
                                                        peer.add_ban_score(20);
                                                        if peer.should_ban() {
                                                            // We can remove directly here as we re-acquired lock
                                                            // and we are outside the main loop lock scope for 'peer'
                                                            // Wait, 'peers' variable shadows outer 'peers'?
                                                            // Yes, 'let mut peers = ...'.
                                                            // So we can remove.
                                                            peers.remove(&id);
                                                        }
                                                    }
                                                    // continue; // Loop continue
                                                } else {
                                                    // Valid payload processing...
                                                    // Re-acquire lock for rate limits
                                                    let mut peers = peers_clone.lock().unwrap();
                                                    // Check if peer still exists
                                                    if let Some(peer) = peers.get_mut(&id) {
                                                        match &payload {
                                                            MessagePayload::Inv(_) => {
                                                                let peer_ip =
                                                                    peer.addr.ip().to_string();
                                                                let is_trusted = peer_ip
                                                                    == "45.77.153.141"
                                                                    || peer_ip == "45.77.64.221";
                                                                if !is_trusted
                                                                    && !peer.check_inv_rate()
                                                                {
                                                                    println!(
                                                                        "Peer {} exceeded INV rate",
                                                                        id
                                                                    );
                                                                    peer.add_ban_score(20);
                                                                }
                                                            }
                                                            MessagePayload::GetData(_) => {
                                                                let peer_ip =
                                                                    peer.addr.ip().to_string();
                                                                let is_trusted = peer_ip
                                                                    == "45.77.153.141"
                                                                    || peer_ip == "45.77.64.221";
                                                                if !is_trusted
                                                                    && !peer.check_getdata_rate()
                                                                {
                                                                    println!("Peer {} exceeded GetData rate", id);
                                                                    peer.add_ban_score(20);
                                                                }
                                                            }
                                                            _ => {}
                                                        }

                                                        if peer.should_ban() {
                                                            peers.remove(&id);
                                                            // continue;
                                                        } else {
                                                            // Continue processing
                                                            drop(peers); // Drop again for dispatch

                                                            // ... Dispatch logic ...
                                                            // Need to indent existing dispatch logic or extract it
                                                            // To avoid massive indentation, I'll use a helper or block
                                                            // The existing code has dispatch logic following.
                                                            // I need to wrap it.

                                                            Self::dispatch_message(
                                                                id,
                                                                &payload,
                                                                magic,
                                                                &peers_clone,
                                                                &relay_clone,
                                                                &sync_clone,
                                                                &discovery_clone,
                                                                &keepalive_clone,
                                                                &stats_clone,
                                                                &recovery_clone,
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                println!(
                                                    "Error parsing message from peer {}: {:?}",
                                                    id, e
                                                );
                                                let stats_guard = stats_clone.lock().unwrap();
                                                if let Some(stats) = &*stats_guard {
                                                    stats.record_invalid_message();
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                // No message, continue
                            }
                            Err(e) => {
                                // NLL: `peer` was last used in `match peer.receive()` above;
                                // its borrow on `peers` has ended. Remove the peer while we
                                // still hold `peers`, then drop `peers` before acquiring any
                                // other lock. This avoids the ABBA same-thread deadlock caused
                                // by the old deferred disconnect path (which held `peers` from
                                // the outer lock() and then tried to call peers_clone.lock()
                                // again in the disconnect block).
                                println!("Error receiving from peer {}: {:?}", id, e);
                                let dconn_info =
                                    peers.remove(&id).map(|p| (p.addr.ip(), p.inbound));
                                let remaining = peers.len();
                                drop(peers);
                                {
                                    let stats_guard = stats_clone.lock().unwrap();
                                    if let Some(stats) = &*stats_guard {
                                        stats.record_connection_closed();
                                    }
                                }
                                {
                                    let sg = sync_clone.lock().unwrap();
                                    if let Some(sync) = &*sg {
                                        sync.on_peer_disconnected(id);
                                    }
                                }
                                batcher_clone.lock().unwrap().clear_peer(id);
                                broadcast_cache_clone.lock().unwrap().clear_peer(id);
                                if let Some((ip, inbound)) = dconn_info {
                                    dos_protection_clone
                                        .lock()
                                        .unwrap()
                                        .record_disconnection(id, ip, inbound);
                                    security_clone.lock().unwrap().remove_peer(id);
                                }
                                if remaining == 0 {
                                    let recovery_bg = Arc::clone(&recovery_clone);
                                    thread::spawn(move || {
                                        let recovery_guard = recovery_bg.lock().unwrap();
                                        if let Some(recovery) = &*recovery_guard {
                                            let _ = recovery.recover();
                                        }
                                    });
                                }
                                continue 'peer_loop;
                            }
                        }
                    } else {
                        // Peer not found (removed concurrently?)
                    }
                }

                thread::sleep(Duration::from_millis(10));
            }
        });
    }

    pub fn broadcast(&self, message: &NetworkMessage) -> Result<(), String> {
        // Collect dead peers while holding peers lock, then release before
        // acquiring batcher/cache/security — same lock-ordering rule as the
        // disconnect path in start_message_handler.
        let dead_peers: Vec<(u64, std::net::IpAddr, bool)> = {
            let mut peers = self.peers.lock().unwrap();
            let mut dead_ids: Vec<u64> = Vec::new();
            for (id, peer) in peers.iter_mut() {
                if peer.state == PeerState::Active && peer.send(message).is_err() {
                    dead_ids.push(*id);
                }
            }
            dead_ids
                .into_iter()
                .filter_map(|id| peers.remove(&id).map(|p| (id, p.addr.ip(), p.inbound)))
                .collect()
        };
        // peers lock released — safe to acquire batcher/cache/security.
        for (id, ip, inbound) in dead_peers {
            self.dos_protection
                .lock()
                .unwrap()
                .record_disconnection(id, ip, inbound);
            self.batcher.lock().unwrap().clear_peer(id);
            self.broadcast_cache.lock().unwrap().clear_peer(id);
            self.security.lock().unwrap().remove_peer(id);
        }

        Ok(())
    }

    pub fn broadcast_inventory(&self, item: InvVector) {
        let hash = item.hash;
        let mut cache = self.broadcast_cache.lock().unwrap();
        let mut batcher = self.batcher.lock().unwrap();

        let peers = self.peers.lock().unwrap();
        for (&peer_id, _) in peers.iter() {
            if !cache.already_sent(peer_id, &hash) {
                batcher.add_inv(peer_id, item);
                cache.mark_sent(peer_id, hash);
            }
        }
    }

    pub fn get_peer(&self, peer_id: u64) -> Option<PeerInfo> {
        let peers = self.peers.lock().unwrap();
        peers.get(&peer_id).map(|p| PeerInfo {
            id: p.id,
            addr: p.addr,
            state: p.state,
            version: p.version,
            services: p.services,
            start_height: p.start_height,
            inbound: p.inbound,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::message::REGTEST_MAGIC;

    #[test]
    fn test_peer_manager_lifecycle() {
        let manager = PeerManager::new(REGTEST_MAGIC, 10, 70015, 0, 0);

        // Start listener
        // Use a random port (0) but we can't easily know it without changing signature.
        // Just verify it doesn't panic.
        assert!(manager
            .start_listener("127.0.0.1:0".parse().unwrap())
            .is_ok());

        assert_eq!(manager.get_peer_count(), 0);
    }

    #[test]
    fn test_peer_limits() {
        let _manager = PeerManager::new(REGTEST_MAGIC, 1, 70015, 0, 0);

        // Mocking connections would require more infrastructure.
        // We can verify limit logic by manually inserting peers if we exposed internals,
        // but since we don't, we rely on integration tests.
        // Assuming implementation is correct based on logic review.
    }
}
