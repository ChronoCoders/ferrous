use ferrous_node::consensus::chain::ChainState;
use ferrous_node::consensus::params::Network;
use ferrous_node::consensus::transaction::Transaction;
use ferrous_node::network::listener::NetworkListener;
use ferrous_node::network::manager::PeerManager;
use ferrous_node::network::mempool::NetworkMempool;
use ferrous_node::network::message::REGTEST_MAGIC;
use ferrous_node::network::relay::BlockRelay;
use ferrous_node::network::sync::SyncManager;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

#[allow(dead_code)]
struct TestNode {
    chain: Arc<Mutex<ChainState>>,
    peer_manager: Arc<PeerManager>,
    relay: Arc<BlockRelay>,
    sync: Arc<SyncManager>,
    mempool: Arc<NetworkMempool>,
    _db_dir: tempfile::TempDir,
}

impl TestNode {
    fn new() -> Self {
        let db_dir = tempdir().unwrap();
        let db_path = db_dir.path().to_str().unwrap();
        let params = Network::Regtest.params();

        let chain = Arc::new(Mutex::new(ChainState::new(params, db_path).unwrap()));

        let peer_manager = Arc::new(PeerManager::new(
            REGTEST_MAGIC,
            10, // Max peers
            70015,
            0,
            0,
        ));

        let mempool = Arc::new(NetworkMempool::new(chain.clone()));

        let relay = Arc::new(BlockRelay::new(
            chain.clone(),
            peer_manager.clone(),
            mempool.clone(),
        ));

        let sync = Arc::new(SyncManager::new(chain.clone(), peer_manager.clone()));

        // Link components
        peer_manager.set_relay(relay.clone());
        peer_manager.set_sync_manager(sync.clone());

        // Start message loop
        peer_manager.start_message_handler();

        Self {
            chain,
            peer_manager,
            relay,
            sync,
            mempool,
            _db_dir: db_dir,
        }
    }

    fn start_listening(&self) -> std::net::SocketAddr {
        let addr_str = "127.0.0.1:0".to_string();
        let temp_listener = NetworkListener::new(addr_str.parse().unwrap(), REGTEST_MAGIC, 10);
        let tcp_listener = temp_listener.bind().unwrap();
        let addr = tcp_listener.local_addr().unwrap();
        drop(tcp_listener);

        self.peer_manager.start_listener(addr).unwrap();
        addr
    }

    fn connect_to(&self, addr: std::net::SocketAddr) {
        self.peer_manager.connect_to_peer(addr).unwrap();
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
}

#[test]
fn test_peer_handshake() {
    let node_a = TestNode::new();
    let node_b = TestNode::new();

    let addr_a = node_a.start_listening();
    node_b.connect_to(addr_a);

    assert!(node_a.wait_for_peers(1, Duration::from_secs(5)));
    assert!(node_b.wait_for_peers(1, Duration::from_secs(5)));
}

#[test]
fn test_transaction_relay() {
    let node_a = TestNode::new();
    let node_b = TestNode::new();

    let addr_a = node_a.start_listening();
    node_b.connect_to(addr_a);

    assert!(node_a.wait_for_peers(1, Duration::from_secs(5)));

    // Create a transaction
    let tx = Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![],
        witnesses: vec![],
        locktime: 0,
    };

    // Add to A
    node_a.mempool.add_transaction(tx.clone()).unwrap();
    node_a.relay.announce_transaction(tx.txid()).unwrap();

    // Wait for B to receive
    let start = std::time::Instant::now();
    let mut received = false;
    while start.elapsed() < Duration::from_secs(5) {
        if node_b.mempool.has_transaction(&tx.txid()) {
            received = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    assert!(received, "Transaction did not propagate to Node B");
}
