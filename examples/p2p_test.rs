use ferrous_node::network::listener::NetworkListener;
use ferrous_node::network::manager::PeerManager;
use ferrous_node::network::message::REGTEST_MAGIC; // Keep for finding port

fn main() {
    let magic = REGTEST_MAGIC;

    // Create PeerManager
    let manager = PeerManager::new(magic, 10, 70015, 1, 0);

    // We need to find a free port first because PeerManager::start_listener
    // doesn't return the bound address/port if we use port 0.
    // So let's bind temporarily to get a port.
    let temp_listener = NetworkListener::new("127.0.0.1:0".parse().unwrap(), magic, 10);
    let tcp_listener = temp_listener.bind().expect("Failed to bind temp listener");
    let local_addr = tcp_listener.local_addr().unwrap();
    drop(tcp_listener); // Release port

    // Start listening
    manager
        .start_listener(local_addr)
        .expect("Failed to start listener");

    println!("Listening on {}", local_addr);
    println!("Peer count: {}", manager.get_peer_count());

    // Start a second peer manager to act as a client
    let client_manager = PeerManager::new(magic, 10, 70015, 1, 0);

    println!("Connecting client to {}...", local_addr);
    match client_manager.connect_to_peer(local_addr) {
        Ok(id) => println!("Client initiated connection, peer id: {}", id),
        Err(e) => println!("Client failed to connect: {}", e),
    }

    // Wait for handshake to complete
    for _ in 0..5 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        println!(
            "Server Active peers: {}/{}",
            manager.active_peer_count(),
            manager.get_peer_count()
        );
        println!(
            "Client Active peers: {}/{}",
            client_manager.active_peer_count(),
            client_manager.get_peer_count()
        );

        if manager.active_peer_count() > 0 && client_manager.active_peer_count() > 0 {
            println!("Connection established and handshake complete!");
            break;
        }
    }

    println!("Test finished.");
}
