use ferrous_node::network::listener::NetworkListener;
use ferrous_node::network::manager::PeerManager;
use ferrous_node::network::message::{NetworkMessage, REGTEST_MAGIC};
use std::thread;
use std::time::Duration;

fn create_test_manager(max_peers: usize) -> PeerManager {
    PeerManager::new(REGTEST_MAGIC, max_peers, 70001, 1, 0)
}

fn wait_for_active_peers(manager: &PeerManager, expected: usize, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if manager.active_peer_count() == expected {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

fn wait_for_peers(manager: &PeerManager, expected: usize, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if manager.peer_count() == expected {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn test_multi_peer_connections() {
    // Create server
    let server = create_test_manager(10);
    // Find a free port
    let temp_listener = NetworkListener::new("127.0.0.1:0".parse().unwrap(), REGTEST_MAGIC, 10);
    let tcp_listener = temp_listener.bind().unwrap();
    let addr = tcp_listener.local_addr().unwrap();
    drop(tcp_listener);

    server.start_listener(addr).unwrap();

    thread::sleep(Duration::from_millis(100));

    // Create 5 clients
    let mut clients = vec![];
    for _ in 0..5 {
        let client = create_test_manager(10);
        let id = client.connect_to_peer(addr).unwrap();
        clients.push((client, id));
        thread::sleep(Duration::from_millis(50));
    }

    // Verify connections - wait for handshakes to complete
    assert!(wait_for_active_peers(&server, 5, Duration::from_secs(5)));
    assert_eq!(server.peer_count(), 5);

    for (client, _) in &clients {
        assert!(wait_for_active_peers(client, 1, Duration::from_secs(5)));
    }
}

#[test]
fn test_peer_limits() {
    let server = create_test_manager(3); // Max 3 peers
    let temp_listener = NetworkListener::new("127.0.0.1:0".parse().unwrap(), REGTEST_MAGIC, 10);
    let tcp_listener = temp_listener.bind().unwrap();
    let addr = tcp_listener.local_addr().unwrap();
    drop(tcp_listener);

    server.start_listener(addr).unwrap();
    thread::sleep(Duration::from_millis(100));

    // Connect 5 clients
    let mut clients = vec![];
    for _ in 0..5 {
        let client = create_test_manager(10);
        let _ = client.connect_to_peer(addr); // might fail if full
        clients.push(client);
        thread::sleep(Duration::from_millis(50));
    }

    // Wait for stabilization
    assert!(wait_for_peers(&server, 3, Duration::from_secs(2)));

    // Should have max 3 peers
    let count = server.peer_count();
    assert!(count <= 3, "Peer count {} exceeded limit 3", count);

    // Verify at least 3 connected (might be flaky if connection rejected fast)
    // The listener rejects immediately if full, so we expect exactly 3 if 5 tried
    assert_eq!(count, 3);
}

#[test]
fn test_broadcast_message() {
    let server = create_test_manager(10);
    let temp_listener = NetworkListener::new("127.0.0.1:0".parse().unwrap(), REGTEST_MAGIC, 10);
    let tcp_listener = temp_listener.bind().unwrap();
    let addr = tcp_listener.local_addr().unwrap();
    drop(tcp_listener);

    server.start_listener(addr).unwrap();

    let mut clients = vec![];
    for _ in 0..3 {
        let client = create_test_manager(10);
        let _ = client.connect_to_peer(addr);
        clients.push(client);
    }

    assert!(wait_for_active_peers(&server, 3, Duration::from_secs(5)));

    let msg = NetworkMessage::new(REGTEST_MAGIC, "ping", vec![1, 2, 3, 4]);
    assert!(server.broadcast(&msg).is_ok());
}

#[test]
fn test_disconnect_peer() {
    let server = create_test_manager(10);
    let temp_listener = NetworkListener::new("127.0.0.1:0".parse().unwrap(), REGTEST_MAGIC, 10);
    let tcp_listener = temp_listener.bind().unwrap();
    let addr = tcp_listener.local_addr().unwrap();
    drop(tcp_listener);

    server.start_listener(addr).unwrap();

    let mut clients = vec![];
    for _ in 0..5 {
        let client = create_test_manager(10);
        let id = client.connect_to_peer(addr).unwrap();
        clients.push((client, id));
    }

    assert!(wait_for_active_peers(&server, 5, Duration::from_secs(5)));

    // Disconnect 2 peers from server side
    clients[0].0.disconnect_peer(clients[0].1).unwrap();
    clients[1].0.disconnect_peer(clients[1].1).unwrap();

    let msg = NetworkMessage::new(REGTEST_MAGIC, "ping", vec![]);
    // Broadcast multiple times to trigger write errors
    for _ in 0..5 {
        let _ = server.broadcast(&msg);
        thread::sleep(Duration::from_millis(50));
    }
}
