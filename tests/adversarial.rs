use ferrous_node::network::manager::PeerManager;
use ferrous_node::network::message::REGTEST_MAGIC;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[test]
fn test_rapid_connect_disconnect() {
    let victim = Arc::new(Mutex::new(PeerManager::new(REGTEST_MAGIC, 50, 70015, 0, 0)));
    let addr: SocketAddr = "127.0.0.1:18450".parse().unwrap();

    let v = Arc::clone(&victim);
    thread::spawn(move || {
        v.lock().unwrap().start_listener(addr).unwrap();
    });

    thread::sleep(Duration::from_millis(100));

    // Rapidly connect and disconnect
    for _ in 0..20 {
        let attacker = PeerManager::new(REGTEST_MAGIC, 10, 70015, 0, 0);
        attacker.connect_to_peer(addr).ok();
        thread::sleep(Duration::from_millis(10));
        // Connection drops when attacker goes out of scope
    }

    thread::sleep(Duration::from_millis(500));

    // Victim should still be functional
    let test_peer = PeerManager::new(REGTEST_MAGIC, 10, 70015, 0, 0);
    assert!(test_peer.connect_to_peer(addr).is_ok());
}

#[test]
fn test_connection_slot_exhaustion() {
    // Max peers = 50
    let victim = Arc::new(Mutex::new(PeerManager::new(REGTEST_MAGIC, 50, 70015, 0, 0)));
    let addr: SocketAddr = "127.0.0.1:18451".parse().unwrap();

    let v = Arc::clone(&victim);
    thread::spawn(move || {
        v.lock().unwrap().start_listener(addr).unwrap();
    });

    thread::sleep(Duration::from_millis(100));

    // Try to exhaust connection slots
    let mut attackers = vec![];
    // Try 60 connections
    for _ in 0..60 {
        let handle = thread::spawn(move || {
            let attacker = PeerManager::new(REGTEST_MAGIC, 10, 70015, 0, 0);
            attacker.connect_to_peer(addr).ok();
            thread::sleep(Duration::from_secs(1));
        });
        attackers.push(handle);
        thread::sleep(Duration::from_millis(10));
    }

    thread::sleep(Duration::from_millis(1000));

    // Should hit limits and reject some
    let count = victim.lock().unwrap().get_peer_count();
    assert!(
        count <= 50,
        "Should enforce max connection limit (actual: {})",
        count
    );

    for handle in attackers {
        handle.join().ok();
    }
}

#[test]
fn test_same_ip_flooding() {
    let victim = Arc::new(Mutex::new(PeerManager::new(REGTEST_MAGIC, 50, 70015, 0, 0)));
    let addr: SocketAddr = "127.0.0.1:18452".parse().unwrap();

    let v = Arc::clone(&victim);
    thread::spawn(move || {
        v.lock().unwrap().start_listener(addr).unwrap();
    });

    thread::sleep(Duration::from_millis(100));

    // Try to connect multiple times from same IP (localhost)
    let mut connections = vec![];
    for _ in 0..10 {
        let handle = thread::spawn(move || {
            let attacker = PeerManager::new(REGTEST_MAGIC, 10, 70015, 0, 0);
            attacker.connect_to_peer(addr).ok();
            thread::sleep(Duration::from_secs(1));
        });
        connections.push(handle);
        thread::sleep(Duration::from_millis(50));
    }

    thread::sleep(Duration::from_millis(500));

    // Should limit connections from same IP
    let count = victim.lock().unwrap().get_peer_count();
    // DoS protection usually limits same IP to 5 or similar small number
    assert!(
        count <= 8,
        "Should limit same-IP connections (actual: {})",
        count
    );

    for handle in connections {
        handle.join().ok();
    }
}
