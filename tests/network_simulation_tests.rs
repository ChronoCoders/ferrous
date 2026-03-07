use ferrous_node::consensus::block::{Block, BlockHeader};
use ferrous_node::consensus::chain::ChainState;
use ferrous_node::consensus::merkle::compute_merkle_root;
use ferrous_node::consensus::params::Network;
use ferrous_node::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
use ferrous_node::mining::miner::{Miner, MiningEvent};
use ferrous_node::network::listener::NetworkListener;
use ferrous_node::network::manager::PeerManager;
use ferrous_node::network::mempool::NetworkMempool;
use ferrous_node::network::message::REGTEST_MAGIC;
use ferrous_node::network::relay::BlockRelay;
use ferrous_node::network::sync::SyncManager;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

struct TestNode {
    pub chain: Arc<Mutex<ChainState>>,
    pub peer_manager: Arc<PeerManager>,
    pub relay: Arc<BlockRelay>,
    pub sync: Arc<SyncManager>,
    pub _mempool: Arc<NetworkMempool>,
    pub miner: Arc<Miner>,
    pub _mining_events: Option<Receiver<MiningEvent>>,
    _db_dir: TempDir,
    pub listen_addr: Option<std::net::SocketAddr>,
    pub name: String,
}

fn make_genesis() -> Block {
    let timestamp = 1296688602;
    let n_bits = 0x207fffff;

    let coinbase = Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig: vec![0x01],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value: 50 * 100_000_000,
            script_pubkey: vec![0x51],
        }],
        witnesses: vec![Witness {
            stack_items: Vec::new(),
        }],
        locktime: 0,
    };

    let txs = vec![coinbase];
    let txids: Vec<_> = txs.iter().map(|tx| tx.txid()).collect();
    let merkle_root = compute_merkle_root(&txids);

    let header = BlockHeader {
        version: 1,
        prev_block_hash: [0u8; 32],
        merkle_root,
        timestamp,
        n_bits,
        nonce: 2, // Nonce 2 works for regtest max target usually
    };

    Block {
        header,
        transactions: txs,
    }
}

impl TestNode {
    fn new(name: &str) -> Self {
        let _ = env_logger::builder().is_test(true).try_init();
        let db_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = db_dir.path();
        let params = Network::Regtest.params();

        // Initialize ChainState
        let mut chain_state =
            ChainState::new(params.clone(), db_path).expect("Failed to create ChainState");

        // Add genesis if empty
        if chain_state.get_tip().unwrap().is_none() {
            let genesis = make_genesis();
            chain_state
                .add_block(genesis)
                .expect("Failed to add genesis block");
        }

        let chain = Arc::new(Mutex::new(chain_state));

        // Initialize PeerManager
        let peer_manager = Arc::new(PeerManager::new(
            REGTEST_MAGIC,
            10,    // Max peers
            70015, // Version
            0,     // Services
            0,     // Height (start at 0)
        ));

        // Initialize Mempool
        let mempool = Arc::new(NetworkMempool::new(chain.clone()));

        // Initialize Relay
        let relay = Arc::new(BlockRelay::new(
            chain.clone(),
            peer_manager.clone(),
            mempool.clone(),
        ));

        // Initialize SyncManager
        let sync = Arc::new(SyncManager::new(chain.clone(), peer_manager.clone()));

        // Link components
        peer_manager.set_relay(relay.clone());
        peer_manager.set_sync_manager(sync.clone());

        // Start message loop
        peer_manager.start_message_handler();

        // Initialize Miner
        let (tx, rx) = channel();
        // Use name to create unique address
        let mut address = vec![0u8; 20];
        let name_bytes = name.as_bytes();
        for (i, b) in name_bytes.iter().enumerate().take(20) {
            address[i] = *b;
        }

        let miner = Miner::new(params, address) // Unique mining address
            .with_event_sender(tx);
        let miner = Arc::new(miner);

        Self {
            chain,
            peer_manager,
            relay,
            sync,
            _mempool: mempool,
            miner,
            _mining_events: Some(rx),
            _db_dir: db_dir,
            listen_addr: None,
            name: name.to_string(),
        }
    }

    fn start_listening(&mut self) -> std::net::SocketAddr {
        let addr_str = "127.0.0.1:0".to_string();
        let temp_listener = NetworkListener::new(addr_str.parse().unwrap(), REGTEST_MAGIC, 10);
        let tcp_listener = temp_listener.bind().unwrap();
        let addr = tcp_listener.local_addr().unwrap();

        drop(tcp_listener); // Release port so PeerManager can bind it
        thread::sleep(Duration::from_millis(50));

        self.peer_manager
            .start_listener(addr)
            .expect("Failed to start listener");
        self.listen_addr = Some(addr);
        addr
    }

    fn connect_to(&self, addr: std::net::SocketAddr) {
        self.peer_manager
            .connect_to_peer(addr)
            .expect("Failed to initiate connection");
    }

    fn wait_for_peers(&self, count: usize, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if self.peer_manager.active_peer_count() >= count {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn mine_block(&self) -> Result<[u8; 32], String> {
        let mut chain = self.chain.lock().unwrap();
        // Use empty transactions for now
        let header = self
            .miner
            .mine_and_attach(&mut chain, vec![])
            .map_err(|e| format!("{:?}", e))?;
        let hash = header.hash();
        drop(chain); // Release lock before announcing

        self.relay.announce_block(hash).map_err(|e| e.to_string())?;
        Ok(hash)
    }

    fn get_tip_height(&self) -> u32 {
        let chain = self.chain.lock().unwrap();
        chain.get_tip().unwrap().unwrap().height as u32
    }

    fn get_tip_hash(&self) -> [u8; 32] {
        let chain = self.chain.lock().unwrap();
        chain.get_tip().unwrap().unwrap().block.header.hash()
    }

    fn wait_for_height(&self, height: u32, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        let mut last_height = 0; // Assuming 0 is starting point or irrelevant
        while start.elapsed() < timeout {
            let current = self.get_tip_height();
            if current != last_height {
                println!("Node {} height: {}", self.name, current);
                last_height = current;
            }
            if current >= height {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        println!(
            "Node {} timeout at height {}",
            self.name,
            self.get_tip_height()
        );
        false
    }

    fn get_peer_id(&self, addr: std::net::SocketAddr) -> Option<u64> {
        let peers_map = self.peer_manager.get_peers();
        let peers = peers_map.lock().unwrap();
        peers
            .iter()
            .find(|(_, p)| p.addr == addr)
            .map(|(id, _)| *id)
    }

    fn trigger_sync(&self, addr: std::net::SocketAddr) {
        // Retry a few times as peer might not be in map immediately after connect returns
        for _ in 0..5 {
            if let Some(id) = self.get_peer_id(addr) {
                self.sync.start_sync(id).expect("Failed to start sync");
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        println!("Peer {} not found for sync", addr);
    }

    fn disconnect_peer_by_addr(&self, addr: std::net::SocketAddr) {
        let peers_map = self.peer_manager.get_peers();
        let mut peers = peers_map.lock().unwrap();
        let id = peers
            .iter()
            .find(|(_, p)| p.addr == addr)
            .map(|(id, _)| *id);
        if let Some(id) = id {
            peers.remove(&id);
        }
    }
}

#[test]
fn test_two_node_mining_propagation() {
    // 1. Setup two nodes
    let mut node_a = TestNode::new("NodeA");
    let node_b = TestNode::new("NodeB");

    // 2. Start networking
    let addr_a = node_a.start_listening();
    node_b.connect_to(addr_a);

    // 3. Wait for connection
    assert!(
        node_a.wait_for_peers(1, Duration::from_secs(5)),
        "Node A failed to find peers"
    );
    assert!(
        node_b.wait_for_peers(1, Duration::from_secs(5)),
        "Node B failed to find peers"
    );

    // 4. Mine 5 blocks on Node A
    println!("Mining 5 blocks on Node A...");
    for i in 1..=5 {
        node_a.mine_block().expect("Failed to mine block");
        println!("Node A mined block at height {}", i);
    }

    // 5. Wait for Node B to sync
    println!("Waiting for Node B to sync...");
    assert!(
        node_b.wait_for_height(5, Duration::from_secs(10)),
        "Node B failed to sync to height 5"
    );

    // Verify tips match
    assert_eq!(
        node_a.get_tip_hash(),
        node_b.get_tip_hash(),
        "Tips do not match after Node A mining"
    );

    // 6. Mine 5 blocks on Node B
    println!("Mining 5 blocks on Node B...");
    for i in 6..=10 {
        node_b.mine_block().expect("Failed to mine block");
        println!("Node B mined block at height {}", i);
    }

    // 7. Wait for Node A to sync
    println!("Waiting for Node A to sync...");
    assert!(
        node_a.wait_for_height(10, Duration::from_secs(10)),
        "Node A failed to sync to height 10"
    );

    // Verify tips match
    assert_eq!(
        node_a.get_tip_hash(),
        node_b.get_tip_hash(),
        "Tips do not match after Node B mining"
    );
}

#[test]
fn test_network_partition_recovery() {
    // 1. Setup 3 nodes: A <-> B <-> C
    let mut node_a = TestNode::new("NodeA");
    let mut node_b = TestNode::new("NodeB");
    let mut node_c = TestNode::new("NodeC");

    let _addr_a = node_a.start_listening();
    let addr_b = node_b.start_listening();
    let addr_c = node_c.start_listening();

    // Connect A -> B
    node_a.connect_to(addr_b);
    // Connect B -> C
    node_b.connect_to(addr_c);

    // Wait for connections
    assert!(node_a.wait_for_peers(1, Duration::from_secs(5)));
    assert!(node_b.wait_for_peers(2, Duration::from_secs(5))); // Connected to A and C
    assert!(node_c.wait_for_peers(1, Duration::from_secs(5)));

    // 2. Mine 5 blocks on A, ensure everyone syncs
    println!("Mining 5 blocks on A (Initial sync)...");
    for _ in 0..5 {
        node_a.mine_block().unwrap();
    }

    assert!(node_b.wait_for_height(5, Duration::from_secs(10)));
    assert!(node_c.wait_for_height(5, Duration::from_secs(10)));

    // 3. Partition: Disconnect B from C
    println!("Partitioning network: B <-> C");
    // Remove C from B's peers
    node_b.disconnect_peer_by_addr(addr_c);
    // Remove B from C's peers
    // We need to remove B from C's peer list.
    // Since C only has one peer (B), we can just clear it.
    {
        let peers_map = node_c.peer_manager.get_peers();
        let mut peers = peers_map.lock().unwrap();
        peers.clear();
    }

    // 4. Mine on Partition 1 (A-B)
    println!("Mining 5 blocks on A (Partition 1)...");
    for _ in 0..5 {
        node_a.mine_block().unwrap();
    }
    // A and B should be at 10
    assert!(node_b.wait_for_height(10, Duration::from_secs(10)));
    // C should still be at 5
    assert_eq!(node_c.get_tip_height(), 5);

    // 5. Mine on Partition 2 (C) - Longer chain
    println!("Mining 10 blocks on C (Partition 2)...");
    for _ in 0..10 {
        node_c.mine_block().unwrap();
    }
    // C should be at 15
    assert_eq!(node_c.get_tip_height(), 15);

    // 6. Heal Partition: Reconnect B -> C
    println!("Healing partition: Reconnecting B -> C");
    node_b.connect_to(addr_c);

    // Wait for connection
    thread::sleep(Duration::from_millis(500));

    // Trigger Sync manually
    println!("Triggering sync B -> C");
    node_b.trigger_sync(addr_c);

    // 7. Verify Re-org
    println!("Waiting for B to re-org...");
    // B should switch to C's chain (height 15)
    assert!(node_b.wait_for_height(15, Duration::from_secs(20)));

    // Give B some time to settle and update its tip
    thread::sleep(Duration::from_secs(2));

    println!("Triggering sync A -> B");
    node_a.trigger_sync(addr_b);

    println!("Waiting for A to re-org...");
    assert!(node_a.wait_for_height(15, Duration::from_secs(20)));

    // Verify all tips match
    let tip_hash = node_c.get_tip_hash();
    assert_eq!(node_b.get_tip_hash(), tip_hash);
    assert_eq!(node_a.get_tip_hash(), tip_hash);
}

#[test]
fn test_concurrent_mining_resolution() {
    // 1. Setup two nodes
    let mut node_a = TestNode::new("NodeA");
    let mut node_b = TestNode::new("NodeB");

    let addr_a = node_a.start_listening();
    let _addr_b = node_b.start_listening();

    // Do NOT connect yet

    // 2. Mine block 1 on A
    node_a.mine_block().unwrap();
    assert_eq!(node_a.get_tip_height(), 1);

    // 3. Mine block 1 on B
    node_b.mine_block().unwrap();
    assert_eq!(node_b.get_tip_height(), 1);

    // Tips should be different (random nonce)
    let tip_a = node_a.get_tip_hash();
    let tip_b = node_b.get_tip_hash();
    assert_ne!(tip_a, tip_b, "Tips should diverge");

    // 4. Mine block 2 on A (Winner chain)
    node_a.mine_block().unwrap();
    assert_eq!(node_a.get_tip_height(), 2);

    // 5. Connect A <-> B
    node_b.connect_to(addr_a);
    assert!(node_b.wait_for_peers(1, Duration::from_secs(5)));

    // 6. Trigger sync B -> A
    node_b.trigger_sync(addr_a);

    // 7. Verify B switches to A's chain
    assert!(node_b.wait_for_height(2, Duration::from_secs(10)));
    assert_eq!(node_b.get_tip_hash(), node_a.get_tip_hash());
}

#[test]
fn test_multi_hop_relay() {
    // A -> B -> C (A not connected to C)
    let mut node_a = TestNode::new("NodeA");
    let mut node_b = TestNode::new("NodeB");
    let mut node_c = TestNode::new("NodeC");

    let _addr_a = node_a.start_listening();
    let addr_b = node_b.start_listening();
    let addr_c = node_c.start_listening();

    node_a.connect_to(addr_b);
    node_b.connect_to(addr_c);

    assert!(node_a.wait_for_peers(1, Duration::from_secs(5)));
    assert!(node_b.wait_for_peers(2, Duration::from_secs(5)));
    assert!(node_c.wait_for_peers(1, Duration::from_secs(5)));

    // Mine on A
    node_a.mine_block().unwrap();

    // Verify C gets it (via B)
    assert!(node_c.wait_for_height(1, Duration::from_secs(10)));
    assert_eq!(node_a.get_tip_hash(), node_c.get_tip_hash());
}
