use crate::consensus::chain::ChainState;
use crate::mining::Miner;
use crate::network::diagnostics::NetworkDiagnostics;
use crate::network::manager::PeerManager;
use crate::network::recovery::RecoveryManager;
use crate::network::stats::NetworkStats;
use crate::rpc::methods::*;
use crate::wallet::builder::TransactionBuilder;
use crate::wallet::manager::Wallet;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tiny_http::{Response, Server};

pub struct RpcServer {
    chain: Arc<Mutex<ChainState>>,
    miner: Arc<Miner>,
    wallet: Arc<Mutex<Wallet>>,
    peer_manager: Arc<PeerManager>,
    network_stats: Arc<NetworkStats>,
    recovery_manager: Arc<RecoveryManager>,
    server: Server,
}

impl RpcServer {
    pub fn new(
        chain: Arc<Mutex<ChainState>>,
        miner: Arc<Miner>,
        wallet: Arc<Mutex<Wallet>>,
        peer_manager: Arc<PeerManager>,
        network_stats: Arc<NetworkStats>,
        recovery_manager: Arc<RecoveryManager>,
        addr: &str,
    ) -> Result<Self, String> {
        let server = Server::http(addr).map_err(|e| format!("Failed to start server: {}", e))?;

        Ok(Self {
            chain,
            miner,
            wallet,
            peer_manager,
            network_stats,
            recovery_manager,
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
            "getbestblockhash" => self.getbestblockhash(),
            "getnewaddress" => self.getnewaddress(),
            "getbalance" => self.getbalance(),
            "listunspent" => self.listunspent(),
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
        let mut content = String::new();
        let mut reader = request.as_reader();
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
        let diagnostics = NetworkDiagnostics::new(self.peer_manager.clone());
        let peers = diagnostics.get_peer_info();

        let peer_list: Vec<_> = peers
            .iter()
            .map(|p| {
                json!({
                    "id": p.peer_id,
                    "addr": p.address,
                    "inbound": p.inbound,
                    "conntime": p.connected_duration.as_secs(),
                    "lastsend": p.last_message.as_secs(),
                    "version": p.version,
                    "startingheight": p.start_height,
                    "bytessent": p.bytes_sent,
                    "bytesrecv": p.bytes_received,
                    "banscore": p.ban_score,
                    "pingtime": p.latency.map(|d: Duration| d.as_millis()),
                })
            })
            .collect();

        Ok(serde_json::Value::Array(peer_list))
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

        let response = GetBlockchainInfoResponse {
            chain: "ferrous".to_string(),
            blocks: tip.height,
            headers: tip.height,
            bestblockhash: hex::encode(tip.header.hash()),
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

        let mut chain = self.chain.lock().map_err(|_| "Lock poisoned".to_string())?;

        let mut block_hashes = Vec::new();

        for _ in 0..nblocks {
            let header = self
                .miner
                .mine_and_attach(&mut chain, Vec::new())
                .map_err(|e| format!("Mining failed: {:?}", e))?;

            block_hashes.push(hex::encode(header.hash()));
        }

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

        let mut chain = self
            .chain
            .lock()
            .map_err(|_| "Chain lock failed".to_string())?;

        let mut block_hashes = Vec::new();

        for _ in 0..nblocks {
            let header = self
                .miner
                .mine_and_attach_to(&mut chain, Vec::new(), script.clone())
                .map_err(|e| format!("Mining failed: {:?}", e))?;

            block_hashes.push(hex::encode(header.hash()));
        }

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
        let tip_height = tip.height;

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
                confirmations,
                script_pubkey: hex::encode(&u.script_pubkey),
            });
        }

        let response = ListUnspentResponse { utxos: items };
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
            .map_err(|e| format!("{:?}", e))?
            .ok_or("Block not found".to_string())?;

        let txids: Vec<String> = block
            .transactions
            .iter()
            .map(|tx| hex::encode(tx.txid()))
            .collect();

        let response = GetBlockResponse {
            hash: hex::encode(block.header.hash()),
            height: block.height,
            version: block.header.version,
            merkleroot: hex::encode(block.header.merkle_root),
            time: block.header.timestamp,
            nonce: block.header.nonce,
            bits: format!("{:08x}", block.header.n_bits),
            tx: txids,
        };

        serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))
    }

    fn getbestblockhash(&self) -> Result<Value, String> {
        let chain = self.chain.lock().map_err(|_| "Lock poisoned".to_string())?;

        let tip = chain.get_tip().map_err(|e| format!("{:?}", e))?;
        Ok(json!(hex::encode(tip.header.hash())))
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
