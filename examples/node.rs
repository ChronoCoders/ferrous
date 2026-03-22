use clap::Parser;
use ferrous_node::consensus::chain::ChainState;
use ferrous_node::consensus::params::Network;
use ferrous_node::mining::{Miner, MiningEvent};
use ferrous_node::network::manager::PeerManager;
use ferrous_node::network::mempool::NetworkMempool;
use ferrous_node::network::message::{MAINNET_MAGIC, REGTEST_MAGIC, TESTNET_MAGIC};
use ferrous_node::network::recovery::RecoveryManager;
use ferrous_node::network::relay::BlockRelay;
use ferrous_node::network::stats::NetworkStats;
use ferrous_node::network::sync::SyncManager;
use ferrous_node::rpc::{RpcServer, RpcServerConfig};
use ferrous_node::wallet::address::address_to_script_pubkey;
use ferrous_node::wallet::manager::Wallet;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};

#[derive(Parser, Debug)]
#[command(name = "ferrous-node")]
#[command(about = "Ferrous Network node", long_about = None)]
struct Args {
    /// RPC server address
    #[arg(long, default_value = "127.0.0.1:8332")]
    rpc_addr: String,

    /// P2P server address
    #[arg(long, default_value = "0.0.0.0:8333")]
    p2p_addr: String,

    /// Mining address (hex scriptPubKey)
    /// If not provided, a new address will be generated from the wallet
    #[arg(long)]
    mining_address: Option<String>,

    #[arg(long, default_value = "mainnet")]
    network: String,

    /// Enable dashboard mode
    #[arg(long, default_value = "false")]
    dashboard: bool,

    /// Database path
    #[arg(long, default_value = "./data")]
    datadir: String,

    /// Wallet file path (wallet.dat)
    #[arg(long, default_value = "./wallet.dat")]
    wallet: String,

    /// Connect to a peer
    #[arg(long)]
    connect: Option<String>,

    /// Seed nodes (comma separated)
    #[arg(long, value_delimiter = ',')]
    seed_nodes: Vec<String>,

    /// Enable continuous mining
    #[arg(long, default_value = "false")]
    mine: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    let network = match args.network.as_str() {
        "mainnet" => Network::Mainnet,
        "testnet" => Network::Testnet,
        "regtest" => Network::Regtest,
        _ => Network::Mainnet,
    };

    let params = network.params();

    let db_path = format!("{}/{}", args.datadir, args.network);
    std::fs::create_dir_all(&db_path)?;

    println!("Ferrous Network v0.1.0");
    println!("Network: {}", args.network);
    println!("Data directory: {}", db_path);

    let chain = Arc::new(RwLock::new(ChainState::new(params.clone(), &db_path)?));

    {
        let mut chain_guard = chain.write().unwrap();

        // Check if genesis needed
        if chain_guard.get_height() == 0 && chain_guard.get_tip().unwrap().is_none() {
            println!("Creating genesis block...");
            let genesis = ferrous_node::consensus::block::create_genesis_block();
            chain_guard.add_block(genesis)?;
            println!("Genesis block created");
        }

        if let Some(tip) = chain_guard.get_tip().unwrap() {
            println!(
                "Current tip: {} (height {})",
                hex::encode(tip.block.header.hash()),
                tip.height
            );
        }
    }

    let network_prefix = match network {
        Network::Mainnet => 0x00,
        Network::Testnet | Network::Regtest => 0x6f,
    };

    let magic_bytes = match args.network.as_str() {
        "mainnet" => MAINNET_MAGIC,
        "testnet" => TESTNET_MAGIC,
        "regtest" => REGTEST_MAGIC,
        _ => MAINNET_MAGIC,
    };

    let wallet = Arc::new(Mutex::new(
        Wallet::load(&args.wallet, network_prefix).map_err(std::io::Error::other)?,
    ));

    // Determine mining address
    let mining_addr_str = match args.mining_address {
        Some(addr) => addr,
        None => {
            let mut w = wallet.lock().unwrap();
            let addr = w.generate_address().map_err(std::io::Error::other)?;
            println!("Generated new mining address: {}", addr);
            addr
        }
    };

    let mining_address = address_to_script_pubkey(&mining_addr_str)
        .or_else(|_| hex::decode(&mining_addr_str))
        .unwrap_or_else(|_| {
            eprintln!(
                "Warning: Invalid mining address/script '{}', using default OP_1",
                mining_addr_str
            );
            vec![0x51]
        });

    // Create network components (stubs for example)
    let peer_manager = Arc::new(PeerManager::new(magic_bytes, 8, 70001, 0, 0));

    // Initialize Managers
    let mempool = Arc::new(NetworkMempool::new(chain.clone()));
    let relay = Arc::new(BlockRelay::new(
        chain.clone(),
        peer_manager.clone(),
        mempool.clone(),
    ));
    let sync_manager = Arc::new(SyncManager::new(chain.clone(), peer_manager.clone()));

    peer_manager.set_relay(relay.clone());
    peer_manager.set_sync_manager(sync_manager.clone());
    sync_manager.start_stall_checker();

    // Start P2P listener
    if let Ok(addr) = args.p2p_addr.parse() {
        println!("Starting P2P listener on {}...", addr);
        peer_manager
            .start_listener(addr)
            .map_err(std::io::Error::other)?;
    } else {
        eprintln!("Invalid P2P address: {}", args.p2p_addr);
    }

    // Start message processing loop
    peer_manager.start_message_handler();

    if let Some(connect_addr) = args.connect {
        if let Ok(addr) = connect_addr.parse() {
            println!("Connecting to peer {}...", addr);
            peer_manager
                .connect_to_peer(addr)
                .map_err(std::io::Error::other)?;
        } else {
            eprintln!("Invalid connect address: {}", connect_addr);
        }
    }

    for seed in args.seed_nodes {
        if let Ok(addr) = seed.parse() {
            println!("Connecting to seed node {}...", addr);
            let _ = peer_manager.connect_to_peer(addr);
        } else {
            eprintln!("Invalid seed address: {}", seed);
        }
    }

    let network_stats = Arc::new(NetworkStats::new());
    peer_manager.set_stats(network_stats.clone());
    // Create AddressManager for RecoveryManager
    let addr_manager = Arc::new(Mutex::new(
        ferrous_node::network::addrman::AddressManager::new(1000),
    ));
    let recovery_manager = Arc::new(RecoveryManager::new(
        peer_manager.clone(),
        addr_manager,
        network.clone(),
        chain.clone(),
    ));
    peer_manager.set_recovery(recovery_manager.clone());
    recovery_manager.start();

    if args.dashboard {
        // Dashboard mode
        use ferrous_node::dashboard::{Dashboard, MiningStats};
        use std::thread;

        let stats = Arc::new(Mutex::new(MiningStats::new(args.network.clone())));

        let (event_sender, event_receiver) = mpsc::channel::<MiningEvent>();

        // Initialize Miner
        // Use the existing params
        let miner = Arc::new(
            Miner::new(params.clone(), mining_address.clone()).with_event_sender(event_sender),
        );

        let chain_clone = chain.clone();
        let miner_clone = miner.clone();
        let wallet_clone = wallet.clone();
        let peer_manager_clone = peer_manager.clone();
        let network_stats_clone = network_stats.clone();
        let recovery_manager_clone = recovery_manager.clone();
        let relay_clone = relay.clone();
        let mempool_clone = mempool.clone();
        let rpc_addr = args.rpc_addr.clone();

        thread::spawn(move || {
            let config = RpcServerConfig {
                chain: chain_clone,
                miner: miner_clone,
                wallet: wallet_clone,
                peer_manager: peer_manager_clone,
                network_stats: network_stats_clone,
                recovery_manager: recovery_manager_clone,
                relay: relay_clone,
                mempool: mempool_clone,
                network_prefix,
            };
            let server = RpcServer::new(config, &rpc_addr).unwrap();
            server.run().ok();
        });

        let mut dashboard = Dashboard::new(stats, event_receiver);
        dashboard.run().unwrap();
    } else {
        println!("Starting RPC server on {}...", args.rpc_addr);

        let miner = Arc::new(Miner::new(params, mining_address));

        let recovery_manager_rpc = recovery_manager.clone();
        let config = RpcServerConfig {
            chain: chain.clone(),
            miner: miner.clone(),
            wallet,
            peer_manager: peer_manager.clone(),
            network_stats,
            recovery_manager: recovery_manager_rpc,
            relay: relay.clone(),
            mempool: mempool.clone(),
            network_prefix,
        };

        let server = RpcServer::new(config, &args.rpc_addr).map_err(std::io::Error::other)?;

        if args.mine {
            println!("Starting continuous miner...");
            let chain_mine = chain.clone();
            let miner_mine = miner.clone();
            let relay_mine = relay.clone();
            let recovery_mine = recovery_manager.clone();
            let mempool_mine = mempool.clone();
            std::thread::spawn(move || loop {
                use ferrous_node::consensus::block::Block;
                use ferrous_node::mining::miner::BlockTemplate;

                // Phase 1: build template with read lock — fast (<1ms), no PoW.
                // Pull pending transactions from the mempool so sendtoaddress
                // transactions are included in the next mined block.
                // Read lock is released before PoW starts so RPC, sync, and
                // the block-dispatch-worker can access the chain concurrently.
                let pending_txs = mempool_mine.get_all_transactions();
                let template_result = {
                    let chain_guard = chain_mine.read().unwrap();
                    miner_mine.build_template(&chain_guard, pending_txs)
                };
                // Read lock is now released.
                let template: BlockTemplate = match template_result {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("Mining error (build_template): {:?}", e);
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue;
                    }
                };

                // Phase 2: run PoW — no chain lock held.
                let (header, txs) = match miner_mine.solve_template(template) {
                    Ok(solved) => solved,
                    Err(e) => {
                        eprintln!("Mining error (solve_template): {:?}", e);
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue;
                    }
                };

                // Phase 3: commit with write lock — brief (~add_block duration).
                let result = {
                    let mut chain_guard = chain_mine.write().unwrap();
                    chain_guard.add_block(Block {
                        header,
                        transactions: txs.clone(),
                    })
                };
                // Write lock is now released.

                match result {
                    Ok(_) => {
                        // Remove confirmed transactions from mempool.
                        mempool_mine.remove_block_transactions(&txs);
                        // Notify recovery manager so last_block_age reflects locally
                        // mined blocks, not just P2P-received ones.
                        recovery_mine.on_new_block();
                        let _ = relay_mine.announce_block(header.hash());
                    }
                    Err(e) => {
                        eprintln!("Mining error (add_block): {:?}", e);
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
            });
        }

        println!("Node running. Press Ctrl+C to stop.");
        println!("\nExample commands:");
        println!("  getblockchaininfo");
        println!("  mineblocks 10");
        println!("  getbestblockhash");

        server.run().map_err(std::io::Error::other)?;
    }

    Ok(())
}
