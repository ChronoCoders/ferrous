use clap::Parser;
use ferrous_node::consensus::chain::{ChainError, ChainState};
use ferrous_node::consensus::params::Network;
use ferrous_node::consensus::utxo::UtxoError;
use ferrous_node::mining::{Miner, MiningEvent};
use ferrous_node::network::manager::PeerManager;
use ferrous_node::network::mempool::NetworkMempool;
use ferrous_node::network::message::{MAINNET_MAGIC, REGTEST_MAGIC, TESTNET_MAGIC};
use ferrous_node::network::recovery::RecoveryManager;
use ferrous_node::network::relay::BlockRelay;
use ferrous_node::network::stats::NetworkStats;
use ferrous_node::network::sync::SyncManager;
use ferrous_node::rpc::{RpcServer, RpcServerConfig};
use ferrous_node::wallet::address::{address_to_hash, address_to_script_pubkey};
use ferrous_node::wallet::manager::Wallet;
use rand::RngCore;
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

    /// Mine to a specific bech32m address (e.g. tfrr1.../frr1...).
    /// Takes precedence over --mining-address and the persisted address.
    /// Must be a valid address for the node's network.
    #[arg(long)]
    mine_to: Option<String>,

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

/// Resolves a stable mining address that survives node restarts.
///
/// The address and its key live in the wallet; the chosen address string is
/// cached in `<datadir>/<network>/mining_address` so the same address is reused
/// on every restart instead of minting a fresh one each boot. The cache is
/// honoured only while the wallet still holds the matching key — if the wallet
/// was wiped independently, a new address is generated and re-cached. The cache
/// file is removed together with the chain data on a testnet reset
/// (`rm -rf data/*`), which is intended: a fresh chain may use a fresh address.
fn resolve_mining_address(
    wallet: &Arc<Mutex<Wallet>>,
    persist_path: &std::path::Path,
) -> Result<String, String> {
    if let Ok(cached) = std::fs::read_to_string(persist_path) {
        let cached = cached.trim();
        if !cached.is_empty() && wallet.lock().unwrap().get_private_key(cached).is_some() {
            println!("Using persisted mining address: {}", cached);
            return Ok(cached.to_string());
        }
    }

    let addr = wallet.lock().unwrap().generate_address()?;
    std::fs::write(persist_path, &addr).map_err(|e| {
        format!(
            "Failed to persist mining address to {}: {}",
            persist_path.display(),
            e
        )
    })?;
    println!("Generated new mining address: {}", addr);
    Ok(addr)
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
            let genesis = ferrous_node::consensus::block::create_genesis_block(
                chain_guard.params.genesis_n_bits,
            );
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

    // Load or generate the RPC cookie for HTTP Basic Auth ("cookie:<hex-token>").
    // Reuse the existing cookie on restart so clients do not need to re-read it.
    // Only generate a new cookie if the file does not exist.
    // Failure to read or write is fatal — the node must not start with an unreadable credential.
    let cookie_path = std::path::PathBuf::from(format!("{}/.rpc.cookie", args.datadir));
    let rpc_cookie: String = if cookie_path.exists() {
        std::fs::read_to_string(&cookie_path)
            .map_err(|e| format!("Fatal: could not read existing RPC cookie: {}", e))?
            .trim()
            .to_string()
    } else {
        let mut token = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut token);
        let credential = format!("cookie:{}", hex::encode(token));
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&cookie_path)
                .and_then(|mut f| f.write_all(credential.as_bytes()))
                .map_err(|e| format!("Fatal: could not write RPC cookie: {}", e))?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&cookie_path, &credential)
                .map_err(|e| format!("Fatal: could not write RPC cookie: {}", e))?;
        }
        println!("RPC cookie written to {}", cookie_path.display());
        credential
    };

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

    // Determine mining address. Precedence:
    //   1. --mine-to <bech32m address>  (validated against the node's network)
    //   2. --mining-address <hex|bech32> (legacy, unvalidated)
    //   3. persisted address cached in the datadir (stable across restarts)
    //   4. a freshly generated wallet address (also persisted)
    let mining_addr_str = if let Some(addr) = args.mine_to {
        let (_, prefix) = address_to_hash(&addr).map_err(std::io::Error::other)?;
        if prefix != network_prefix {
            return Err(std::io::Error::other(format!(
                "--mine-to address is for network prefix 0x{:02x}, but this node is 0x{:02x}",
                prefix, network_prefix
            ))
            .into());
        }
        println!("Mining to --mine-to address: {}", addr);
        addr
    } else if let Some(addr) = args.mining_address {
        addr
    } else {
        let persist_path = std::path::Path::new(&db_path).join("mining_address");
        resolve_mining_address(&wallet, &persist_path).map_err(std::io::Error::other)?
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

    // Parse seed nodes and keep the addresses so RecoveryManager can
    // reconnect to them during Stage 2/4 without relying on get_seed_nodes()
    // (which returns an empty list for testnet).
    let mut configured_seeds: Vec<std::net::SocketAddr> = Vec::new();
    for seed in args.seed_nodes {
        if let Ok(addr) = seed.parse::<std::net::SocketAddr>() {
            println!("Connecting to seed node {}...", addr);
            let _ = peer_manager.connect_to_peer(addr);
            configured_seeds.push(addr);
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
        configured_seeds,
    ));
    peer_manager.set_recovery(recovery_manager.clone());
    recovery_manager.set_sync_manager(sync_manager.clone());
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

        let rpc_cookie_dash = rpc_cookie.clone();
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
                rpc_auth: Some(rpc_cookie_dash),
            };
            let server = std::sync::Arc::new(RpcServer::new(config, &rpc_addr).unwrap());
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
            rpc_auth: Some(rpc_cookie),
        };

        let server = std::sync::Arc::new(
            RpcServer::new(config, &args.rpc_addr).map_err(std::io::Error::other)?,
        );

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
                    Ok(requeued_txs) => {
                        // Remove confirmed transactions from mempool.
                        mempool_mine.remove_block_transactions(&txs);
                        mempool_mine.purge_stale();
                        // Re-add transactions from any disconnected blocks.
                        for tx in requeued_txs {
                            let _ = mempool_mine.add_transaction(tx);
                        }
                        // Notify recovery manager so last_block_age reflects locally
                        // mined blocks, not just P2P-received ones.
                        recovery_mine.on_new_block();
                        let _ = relay_mine.announce_block(header.hash());
                    }
                    Err(ChainError::UtxoError(UtxoError::ScriptValidationFailed)) => {
                        eprintln!(
                            "Mining error (add_block): ScriptValidationFailed — clearing mempool"
                        );
                        // purge_stale only checks UTXO existence, not script validity.
                        // A transaction with a valid UTXO but invalid script will loop
                        // forever. Clear the entire mempool to break the cycle.
                        mempool_mine.clear();
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                    Err(e) => {
                        eprintln!("Mining error (add_block): {:?}", e);
                        // Purge stale mempool entries — a reorg may have
                        // invalidated transactions in the template we just tried.
                        mempool_mine.purge_stale();
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
