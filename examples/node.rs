use clap::Parser;
use ferrous_node::consensus::chain::ChainState;
use ferrous_node::consensus::params::Network;
use ferrous_node::mining::{Miner, MiningEvent};
use ferrous_node::network::manager::PeerManager;
use ferrous_node::network::recovery::RecoveryManager;
use ferrous_node::network::stats::NetworkStats;
use ferrous_node::rpc::RpcServer;
use ferrous_node::wallet::manager::Wallet;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

#[derive(Parser, Debug)]
#[command(name = "ferrous-node")]
#[command(about = "Ferrous Network node", long_about = None)]
struct Args {
    /// RPC server address
    #[arg(long, default_value = "127.0.0.1:8332")]
    rpc_addr: String,

    /// Mining address (hex scriptPubKey)
    #[arg(long, default_value = "51")]
    mining_address: String,

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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    let chain = Arc::new(Mutex::new(ChainState::new(params.clone(), &db_path)?));

    {
        let mut chain_guard = chain.lock().unwrap();

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

    let mining_address = hex::decode(&args.mining_address).unwrap_or_else(|_| vec![0x51]);

    println!("Mining address: {}", hex::encode(&mining_address));

    let network_prefix = match network {
        Network::Mainnet => 0x00,
        Network::Testnet | Network::Regtest => 0x6f,
    };

    let wallet = Arc::new(Mutex::new(
        Wallet::load(&args.wallet, network_prefix).map_err(std::io::Error::other)?,
    ));

    // Create network components (stubs for example)
    let peer_manager = Arc::new(PeerManager::new([0u8; 4], 8, 70001, 0, 0));
    let network_stats = Arc::new(NetworkStats::new());
    // Create AddressManager for RecoveryManager
    let addr_manager = Arc::new(Mutex::new(
        ferrous_node::network::addrman::AddressManager::new(1000),
    ));
    let recovery_manager = Arc::new(RecoveryManager::new(peer_manager.clone(), addr_manager));

    if args.dashboard {
        // Dashboard mode
        use ferrous_node::dashboard::{Dashboard, MiningStats};
        use std::thread;

        let stats = Arc::new(Mutex::new(MiningStats::new(args.network.clone())));

        let (event_sender, event_receiver) = mpsc::channel::<MiningEvent>();

        let miner = Arc::new(Miner::new(params, mining_address).with_event_sender(event_sender));

        let chain_clone = chain.clone();
        let miner_clone = miner.clone();
        let wallet_clone = wallet.clone();
        let peer_manager_clone = peer_manager.clone();
        let network_stats_clone = network_stats.clone();
        let recovery_manager_clone = recovery_manager.clone();
        let rpc_addr = args.rpc_addr.clone();

        thread::spawn(move || {
            let server = RpcServer::new(
                chain_clone,
                miner_clone,
                wallet_clone,
                peer_manager_clone,
                network_stats_clone,
                recovery_manager_clone,
                &rpc_addr,
            )
            .unwrap();
            server.run().ok();
        });

        let mut dashboard = Dashboard::new(stats, event_receiver);
        dashboard.run().unwrap();
    } else {
        println!("Starting RPC server on {}...", args.rpc_addr);

        let miner = Arc::new(Miner::new(params, mining_address));

        let server = RpcServer::new(
            chain,
            miner,
            wallet,
            peer_manager,
            network_stats,
            recovery_manager,
            &args.rpc_addr,
        )
        .map_err(std::io::Error::other)?;

        println!("Node running. Press Ctrl+C to stop.");
        println!("\nExample commands:");
        println!("  getblockchaininfo");
        println!("  mineblocks 10");
        println!("  getbestblockhash");

        server.run().map_err(std::io::Error::other)?;
    }

    Ok(())
}
