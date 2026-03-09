use crate::consensus::chain::ChainState;
use crate::mining::Miner;
use crate::network::diagnostics::NetworkDiagnostics;
use crate::network::manager::PeerManager;
use crate::network::relay::BlockRelay;
use crate::network::recovery::RecoveryManager;
use crate::network::stats::NetworkStats;
use crate::rpc::methods::*;
use crate::wallet::builder::TransactionBuilder;
use crate::wallet::manager::Wallet;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::{Arc, Mutex};
use tiny_http::{Response, Server};

pub struct RpcServerConfig {
    pub chain: Arc<Mutex<ChainState>>,
    pub miner: Arc<Miner>,
    pub wallet: Arc<Mutex<Wallet>>,
    pub peer_manager: Arc<PeerManager>,
    pub network_stats: Arc<NetworkStats>,
    pub recovery_manager: Arc<RecoveryManager>,
    pub relay: Arc<BlockRelay>,
}

pub struct RpcServer {
    chain: Arc<Mutex<ChainState>>,
    miner: Arc<Miner>,
    wallet: Arc<Mutex<Wallet>>,
    peer_manager: Arc<PeerManager>,
    network_stats: Arc<NetworkStats>,
    recovery_manager: Arc<RecoveryManager>,
    relay: Arc<BlockRelay>,
    server: Server,
}

impl RpcServer {
    pub fn new(config: RpcServerConfig, addr: &str) -> Result<Self, String> {
        let server = Server::http(addr).map_err(|e| format!("Failed to start server: {}", e))?;

        Ok(Self {
            chain: config.chain,
            miner: config.miner,
            wallet: config.wallet,
            peer_manager: config.peer_manager,
            network_stats: config.network_stats,
            recovery_manager: config.recovery_manager,
            relay: config.relay,
            server,
        })
    }

    pub fn run(&self) -> Result<(), String> {
        for mut request in self.server.incoming_requests() {
            let (response, stop) = self.handle_request(&mut request);
            let _ = request.respond(response);
            if stop {
                break;
            }
        }
        Ok(())
    }

    pub fn handle_raw(&self, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
        let req: Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => return self.error_response(Value::Null, -32700, "Parse error"),
        };

        self.handle_json_rpc(req)
    }

    pub fn handle_json_rpc(&self, req: Value) -> Response<std::io::Cursor<Vec<u8>>> {
        let method = req["method"].as_str().unwrap_or("");
        let params = &req["params"];
        let id = req["id"].clone();

        let result = match method {
            "getblockchaininfo" => self.getblockchaininfo(),
            "mineblocks" => self.mineblocks(params),
            "getblock" => self.getblock(params),
            "getblockhash" => self.getblockhash(params),
            "getbestblockhash" => self.getbestblockhash(),
            "addnode" => self.addnode(params),
            "getnewaddress" => self.getnewaddress(),
            "getbalance" => self.getbalance(),
            "listunspent" => self.listunspent(),
            "listaddresses" => self.listaddresses(),
            "sendtoaddress" => self.sendtoaddress(params),
            "generatetoaddress" => self.generatetoaddress(params),
            "getnetworkinfo" => self.getnetworkinfo(),
            "getpeerinfo" => self.getpeerinfo(),
            "getconnectioncount" => self.getconnectioncount(),
            "getnetworkhealth" => self.getnetworkhealth(),
            "getrecoverystatus" => self.getrecoverystatus(),
            "forcereconnect" => self.forcereconnect(),
            "resetnetwork" => self.resetnetwork(),
            "stop" => Ok(json!("stopping")),
            _ => return self.error_response(id, -32601, "Method not found"),
        };

        match result {
            Ok(v) => self.success_response(id, v),
            Err(e) => self.error_response(id, -32603, &e),
        }
    }

    fn handle_request(
        &self,
        request: &mut tiny_http::Request,
    ) -> (Response<std::io::Cursor<Vec<u8>>>, bool) {
        const MAX_REQUEST_BODY: usize = 1 * 1024 * 1024;

        let content_length = request.body_length().unwrap_or(0);
        if content_length > MAX_REQUEST_BODY {
            return (
                self.error_response(Value::Null, -32600, "Request too large"),
                false,
            );
        }

        let mut content = String::new();
        let mut reader = request.as_reader().take(MAX_REQUEST_BODY as u64);
        if std::io::Read::read_to_string(&mut reader, &mut content).is_err() {
            return (
                self.error_response(Value::Null, -32700, "Parse error"),
                false,
            );
        }

        let is_stop = serde_json::from_str::<Value>(&content)
            .ok()
            .and_then(|v| {
                v.get("method")
                    .and_then(|m| m.as_str().map(|s| s == "stop"))
            })
            .unwrap_or(false);

        (self.handle_raw(&content), is_stop)
    }

    fn getnetworkinfo(&self) -> Result<Value, String> {
        let stats = self.network_stats.get_snapshot();

        Ok(json!({
            "version": 70001,
            "connections": stats.current_connections,
            "connections_in": stats.total_connections_accepted,
            "connections_out": stats.total_connections_initiated,
            "bytes_sent": stats.bytes_sent,
            "bytes_recv": stats.bytes_received,
            "send_rate_mbps": (stats.avg_send_rate * 8.0) / 1_000_000.0,
            "recv_rate_mbps": (stats.avg_recv_rate * 8.0) / 1_000_000.0,
            "uptime": stats.uptime_secs,
        }))
    }

    fn getpeerinfo(&self) -> Result<Value, String> {
        let count = self.peer_manager.get_peer_count();
        let addrs = self.peer_manager.get_peer_addrs();
        
        // Return simple list for now
        let info: Vec<_> = addrs.iter().map(|a| a.to_string()).collect();
        Ok(json!({
            "count": count,
            "peers": info
        }))
    }

    fn getconnectioncount(&self) -> Result<Value, String> {
        let diagnostics = NetworkDiagnostics::new(self.peer_manager.clone());
        let summary = diagnostics.get_connection_summary();
        Ok(json!(summary.total_peers))
    }

    fn getnetworkhealth(&self) -> Result<Value, String> {
        let diagnostics = NetworkDiagnostics::new(self.peer_manager.clone());
        let health_score = diagnostics.get_health_score();

        Ok(json!({
            "health_score": health_score,
            "status": if health_score >= 80 { "excellent" }
                      else if health_score >= 60 { "good" }
                      else if health_score >= 40 { "fair" }
                      else { "poor" },
        }))
    }

    fn getrecoverystatus(&self) -> Result<Value, String> {
        Ok(json!({
            "partition_detected": self.recovery_manager.is_partitioned(),
            "recovery_attempts": self.recovery_manager.get_attempts(),
            "last_block_age": self.recovery_manager.get_last_block_age_secs(),
        }))
    }

    fn forcereconnect(&self) -> Result<Value, String> {
        self.recovery_manager.force_reconnect();
        Ok(json!({"result": "reconnecting"}))
    }

    fn resetnetwork(&self) -> Result<Value, String> {
        match self.recovery_manager.recover() {
            Ok(_) => Ok(json!({"result": "network reset initiated"})),
            Err(e) => Err(format!("Network reset failed: {}", e)),
        }
    }

    fn getblockchaininfo(&self) -> Result<Value, String> {
        let chain = self.chain.lock().map_err(|_| "Lock poisoned".to_string())?;
        let tip = chain.get_tip().map_err(|e| format!("{:?}", e))?;

        let height = tip.as_ref().map(|t| t.height as u32).unwrap_or(0);
        let bestblockhash = tip
            .as_ref()
            .map(|t| hex::encode(t.block.header.hash()))
            .unwrap_or_else(|| "00".repeat(32));

        let response = GetBlockchainInfoResponse {
            chain: "ferrous".to_string(),
            blocks: height,
            headers: height,
            bestblockhash,
        };

        serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))
    }

    fn mineblocks(&self, params: &Value) -> Result<Value, String> {
        let nblocks = params
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_u64())
            .ok_or("Invalid params: expected [nblocks]")?;

        if nblocks == 0 || nblocks > 1000 {
            return Err("nblocks must be between 1 and 1000".to_string());
        }

        let mut block_hashes = Vec::new();
        let mut last_hash = [0u8; 32];

        for _ in 0..nblocks {
            let hash = {
                let mut chain = self.chain.lock().map_err(|_| "Lock poisoned".to_string())?;
                let header = self
                    .miner
                    .mine_and_attach(&mut chain, Vec::new())
                    .map_err(|e| format!("Mining failed: {:?}", e))?;
                header.hash()
            };
            last_hash = hash;
            block_hashes.push(hex::encode(hash));
        }

        let _ = self.relay.announce_block(last_hash);

        let response = MineBlocksResponse {
            blocks: block_hashes,
        };

        serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))
    }

    fn generatetoaddress(&self, params: &Value) -> Result<Value, String> {
        let arr = params
            .as_array()
            .ok_or("Invalid params: expected [nblocks, address]")?;

        let nblocks = arr
            .first()
            .and_then(|v| v.as_u64())
            .ok_or("Missing nblocks parameter")?;

        if nblocks == 0 || nblocks > 1000 {
            return Err("nblocks must be between 1 and 1000".to_string());
        }

        let address = arr
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or("Missing address parameter")?;

        let script = crate::wallet::address::address_to_script_pubkey(address)
            .map_err(|e| format!("Invalid address: {}", e))?;

        let mut block_hashes = Vec::new();
        let mut last_hash = [0u8; 32];

        for _ in 0..nblocks {
            let hash = {
                let mut chain = self
                    .chain
                    .lock()
                    .map_err(|_| "Chain lock failed".to_string())?;
                let header = self
                    .miner
                    .mine_and_attach_to(&mut chain, Vec::new(), script.clone())
                    .map_err(|e| format!("Mining failed: {:?}", e))?;
                header.hash()
            };
            last_hash = hash;
            block_hashes.push(hex::encode(hash));
        }

        let _ = self.relay.announce_block(last_hash);

        let response = MineBlocksResponse {
            blocks: block_hashes,
        };

        serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))
    }

    fn getnewaddress(&self) -> Result<Value, String> {
        let mut wallet = self
            .wallet
            .lock()
            .map_err(|_| "Lock poisoned".to_string())?;
        let address = wallet.generate_address()?;
        let response = GetNewAddressResponse { address };
        serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))
    }

    fn getbalance(&self) -> Result<Value, String> {
        let wallet = self
            .wallet
            .lock()
            .map_err(|_| "Lock poisoned".to_string())?;
        let chain = self.chain.lock().map_err(|_| "Lock poisoned".to_string())?;
        let sats = wallet.get_balance(&chain)?;
        let balance = sats as f64 / 100_000_000f64;
        let response = GetBalanceResponse { balance };
        serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))
    }

    fn listunspent(&self) -> Result<Value, String> {
        let wallet = self
            .wallet
            .lock()
            .map_err(|_| "Lock poisoned".to_string())?;
        let chain = self.chain.lock().map_err(|_| "Lock poisoned".to_string())?;
        let utxos = wallet.get_utxos(&chain)?;
        let tip = chain.get_tip().map_err(|e| format!("{:?}", e))?;
        let tip_height = tip.as_ref().map(|t| t.height).unwrap_or(0);

        let mut items = Vec::new();
        for u in utxos {
            let confirmations = if tip_height >= u.height {
                tip_height - u.height + 1
            } else {
                0
            };
            items.push(ListUnspentItem {
                txid: hex::encode(u.txid),
                vout: u.vout,
                amount: u.value as f64 / 100_000_000f64,
                confirmations: confirmations as u32,
                script_pubkey: hex::encode(&u.script_pubkey),
            });
        }

        let response = ListUnspentResponse { utxos: items };
        serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))
    }

    fn listaddresses(&self) -> Result<Value, String> {
        let wallet = self
            .wallet
            .lock()
            .map_err(|_| "Lock poisoned".to_string())?;
        
        let addresses = wallet.addresses();
        let response = ListAddressesResponse { addresses };
        
        serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))
    }

    fn sendtoaddress(&self, params: &Value) -> Result<Value, String> {
        let addr = params
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or("Invalid params: expected [address, amount]")?;

        let amount = params
            .as_array()
            .and_then(|arr| arr.get(1))
            .and_then(|v| v.as_f64())
            .ok_or("Invalid params: expected [address, amount]")?;

        if amount <= 0.0 {
            return Err("Amount must be positive".to_string());
        }

        let sats = (amount * 100_000_000f64).round() as u64;
        let fee = 1000u64;

        let wallet = self
            .wallet
            .lock()
            .map_err(|_| "Lock poisoned".to_string())?;
        let mut chain = self.chain.lock().map_err(|_| "Lock poisoned".to_string())?;

        let tx = TransactionBuilder::create_transaction(&wallet, &chain, addr, sats, fee)
            .map_err(|e| format!("Transaction creation failed: {}", e))?;

        let header = self
            .miner
            .mine_and_attach(&mut chain, vec![tx.clone()])
            .map_err(|e| format!("Mining failed: {:?}", e))?;

        let txid = hex::encode(tx.txid());
        let blockhash = hex::encode(header.hash());

        let response = SendToAddressResponse { txid, blockhash };
        serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))
    }

    fn getblock(&self, params: &Value) -> Result<Value, String> {
        let blockhash_hex = params
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or("Invalid params: expected [blockhash]")?;

        let blockhash_bytes = hex::decode(blockhash_hex).map_err(|_| "Invalid hex".to_string())?;

        if blockhash_bytes.len() != 32 {
            return Err("Invalid hash length".to_string());
        }

        let mut hash = [0u8; 32];
        hash.copy_from_slice(&blockhash_bytes);

        let chain = self.chain.lock().map_err(|_| "Lock poisoned".to_string())?;

        let block = chain
            .get_block(&hash)
            .ok_or("Block not found".to_string())?;

        let height = chain.get_height_for_hash(&hash).unwrap_or(0) as u32;

        let txids: Vec<String> = block
            .transactions
            .iter()
            .map(|tx| hex::encode(tx.txid()))
            .collect();

        let response = GetBlockResponse {
            hash: hex::encode(block.header.hash()),
            height,
            version: block.header.version,
            merkleroot: hex::encode(block.header.merkle_root),
            time: block.header.timestamp,
            nonce: block.header.nonce,
            bits: format!("{:08x}", block.header.n_bits),
            tx: txids,
        };

        serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))
    }

    fn getblockhash(&self, params: &Value) -> Result<Value, String> {
        let height = params
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_u64())
            .ok_or("Invalid params: expected [height]")?;

        let chain = self.chain.lock().map_err(|_| "Lock poisoned".to_string())?;
        
        let hash = chain
            .get_block_hash(height)
            .ok_or("Block height out of range".to_string())?;

        Ok(json!(hex::encode(hash)))
    }

    fn getbestblockhash(&self) -> Result<Value, String> {
        let chain = self.chain.lock().map_err(|_| "Lock poisoned".to_string())?;
        let tip = chain.get_tip().map_err(|e| format!("{:?}", e))?;
        
        match tip {
            Some(t) => Ok(json!(hex::encode(t.block.header.hash()))),
            None => Ok(json!(null)),
        }
    }

    fn addnode(&self, params: &Value) -> Result<Value, String> {
        let addr_str = params
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or("Invalid params: expected [address]")?;

        let addr: std::net::SocketAddr = addr_str
            .parse()
            .map_err(|e| format!("Invalid address: {}", e))?;

        self.peer_manager
            .connect_to_peer(addr)
            .map_err(|e| format!("Failed to connect: {}", e))?;

        Ok(json!("added"))
    }

    fn success_response(&self, id: Value, result: Value) -> Response<std::io::Cursor<Vec<u8>>> {
        let body = json!({
            "jsonrpc": "2.0",
            "result": result,
            "id": id
        });

        Response::from_string(body.to_string()).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        )
    }

    fn error_response(
        &self,
        id: Value,
        code: i32,
        message: &str,
    ) -> Response<std::io::Cursor<Vec<u8>>> {
        let body = json!({
            "jsonrpc": "2.0",
            "error": {
                "code": code,
                "message": message
            },
            "id": id
        });

        Response::from_string(body.to_string()).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        )
    }
}
