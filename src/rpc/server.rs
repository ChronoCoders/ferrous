use crate::consensus::chain::ChainState;
use crate::mining::Miner;
use crate::network::diagnostics::NetworkDiagnostics;
use crate::network::manager::PeerManager;
use crate::network::mempool::NetworkMempool;
use crate::network::recovery::RecoveryManager;
use crate::network::relay::BlockRelay;
use crate::network::stats::NetworkStats;
use crate::primitives::serialize::Decode;
use crate::rpc::methods::*;
use crate::wallet::builder::TransactionBuilder;
use crate::wallet::manager::Wallet;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tiny_http::{Response, Server};

pub struct RpcServerConfig {
    pub chain: Arc<RwLock<ChainState>>,
    pub miner: Arc<Miner>,
    pub wallet: Arc<Mutex<Wallet>>,
    pub peer_manager: Arc<PeerManager>,
    pub network_stats: Arc<NetworkStats>,
    pub recovery_manager: Arc<RecoveryManager>,
    pub relay: Arc<BlockRelay>,
    pub mempool: Arc<NetworkMempool>,
}

pub struct RpcServer {
    chain: Arc<RwLock<ChainState>>,
    miner: Arc<Miner>,
    wallet: Arc<Mutex<Wallet>>,
    peer_manager: Arc<PeerManager>,
    network_stats: Arc<NetworkStats>,
    recovery_manager: Arc<RecoveryManager>,
    relay: Arc<BlockRelay>,
    mempool: Arc<NetworkMempool>,
    server: Server,
    /// 1-second response cache for getblockchaininfo (timestamp, cached value).
    blockchain_info_cache: Mutex<Option<(Instant, Value)>>,
    /// 1-second response cache for getmininginfo (timestamp, cached value).
    mininginfo_cache: Mutex<Option<(Instant, Value)>>,
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
            mempool: config.mempool,
            server,
            blockchain_info_cache: Mutex::new(None),
            mininginfo_cache: Mutex::new(None),
        })
    }

    pub fn run(&self) -> Result<(), String> {
        for mut request in self.server.incoming_requests() {
            // All chain/wallet locks are acquired and released inside handle_request.
            // respond() is called after handle_request returns, so no lock is held
            // across the network send (Fix 3: lock-free send boundary).
            let (response, stop) = self.handle_request(&mut request);
            let _ = request.respond(response);
            if stop {
                break;
            }
        }
        Ok(())
    }

    pub fn handle_raw(&self, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
        self.handle_json_rpc_body(body.as_bytes()).0
    }

    pub fn handle_json_rpc(&self, req: Value) -> Response<std::io::Cursor<Vec<u8>>> {
        match self.handle_json_rpc_value(&req).0 {
            Some(v) => self.json_response(v),
            None => Response::from_string("").with_status_code(204),
        }
    }

    fn handle_json_rpc_body(&self, body: &[u8]) -> (Response<std::io::Cursor<Vec<u8>>>, bool) {
        let trimmed = self.trim_body(body);
        let req: Value = match serde_json::from_slice(trimmed) {
            Ok(Value::String(s)) => match serde_json::from_str(&s) {
                Ok(v) => v,
                Err(_) => {
                    return (
                        self.error_response(Value::Null, -32700, "Parse error"),
                        false,
                    )
                }
            },
            Ok(v) => v,
            Err(_) => {
                let without_nul: Vec<u8> = trimmed.iter().copied().filter(|b| *b != 0).collect();
                let secondary = if without_nul.len() == trimmed.len() {
                    trimmed
                } else {
                    &without_nul
                };

                match serde_json::from_slice::<Value>(secondary) {
                    Ok(Value::String(s)) => match serde_json::from_str(&s) {
                        Ok(v) => v,
                        Err(_) => {
                            return (
                                self.error_response(Value::Null, -32700, "Parse error"),
                                false,
                            )
                        }
                    },
                    Ok(v) => v,
                    Err(_) => {
                        if let Some(v) = self.try_parse_backslash_escaped_json(secondary) {
                            v
                        } else if let Some(v) = self.try_parse_backslash_quote_json(secondary) {
                            v
                        } else if let Some(v) = self.try_parse_extracted_json(secondary) {
                            v
                        } else {
                            return (
                                self.error_response(Value::Null, -32700, "Parse error"),
                                false,
                            );
                        }
                    }
                }
            }
        };

        match req {
            Value::Array(items) => {
                if items.is_empty() {
                    return (
                        self.error_response(Value::Null, -32600, "Invalid Request"),
                        false,
                    );
                }

                let mut responses: Vec<Value> = Vec::new();
                let mut stop = false;
                for item in &items {
                    let (resp, should_stop) = self.handle_json_rpc_value(item);
                    stop |= should_stop;
                    if let Some(resp) = resp {
                        responses.push(resp);
                    }
                }

                if responses.is_empty() {
                    return (Response::from_string("").with_status_code(204), stop);
                }

                (self.json_response(Value::Array(responses)), stop)
            }
            other => {
                let (resp, stop) = self.handle_json_rpc_value(&other);
                match resp {
                    Some(v) => (self.json_response(v), stop),
                    None => (Response::from_string("").with_status_code(204), stop),
                }
            }
        }
    }

    fn handle_json_rpc_value(&self, req: &Value) -> (Option<Value>, bool) {
        let Some(obj) = req.as_object() else {
            return (
                Some(self.error_value(Value::Null, -32600, "Invalid Request")),
                false,
            );
        };

        let method = obj.get("method").and_then(|m| m.as_str()).unwrap_or("");
        if method.is_empty() {
            return (
                Some(self.error_value(Value::Null, -32600, "Invalid Request")),
                false,
            );
        }

        let stop = method == "stop";

        let id_present = obj.contains_key("id");
        let id = obj.get("id").cloned().unwrap_or(Value::Null);
        if !id_present {
            let _ = self.dispatch_method(method, obj.get("params").unwrap_or(&Value::Null));
            return (None, stop);
        }

        let params = obj.get("params").unwrap_or(&Value::Null);
        match self.dispatch_method(method, params) {
            Ok(result) => (Some(self.success_value(id, result)), stop),
            Err((code, message)) => (Some(self.error_value(id, code, message)), stop),
        }
    }

    fn dispatch_method(&self, method: &str, params: &Value) -> Result<Value, (i32, String)> {
        let result = match method {
            "getblockchaininfo" => self.getblockchaininfo(),
            "getmininginfo" => self.getmininginfo(),
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
            "sendrawtransaction" => self.sendrawtransaction(params),
            "stop" => Ok(json!("stopping")),
            _ => return Err((-32601, "Method not found".to_string())),
        };

        match result {
            Ok(v) => Ok(v),
            Err(e) => Err((-32603, e)),
        }
    }

    fn try_parse_backslash_escaped_json(&self, body: &[u8]) -> Option<Value> {
        let trimmed = self.trim_body(body);
        if !(trimmed.starts_with(b"{\\\"") || trimmed.starts_with(b"[{\\\"")) {
            return None;
        }

        let mut cleaned = Vec::with_capacity(trimmed.len());
        let mut i = 0;
        while i < trimmed.len() {
            if trimmed[i] == b'\\' && i + 1 < trimmed.len() && trimmed[i + 1] == b'"' {
                cleaned.push(b'"');
                i += 2;
                continue;
            }
            cleaned.push(trimmed[i]);
            i += 1;
        }

        serde_json::from_slice(&cleaned).ok()
    }

    fn try_parse_backslash_quote_json(&self, body: &[u8]) -> Option<Value> {
        if body.contains(&b'"') || !body.contains(&b'\\') {
            return None;
        }

        let mut cleaned = Vec::with_capacity(body.len());
        let mut i = 0;
        while i < body.len() {
            if body[i] == b'\\' {
                if i + 1 < body.len() && body[i + 1] == b':' {
                    cleaned.push(b'"');
                    cleaned.push(b':');
                    i += 2;
                    continue;
                }
                if i + 1 < body.len() && body[i + 1] == b',' {
                    cleaned.push(b'"');
                    cleaned.push(b',');
                    i += 2;
                    continue;
                }
                cleaned.push(b'"');
                i += 1;
                continue;
            }
            cleaned.push(body[i]);
            i += 1;
        }

        serde_json::from_slice(&cleaned).ok()
    }

    fn try_parse_extracted_json(&self, body: &[u8]) -> Option<Value> {
        let mut start = None;
        for (i, b) in body.iter().copied().enumerate() {
            if b == b'{' || b == b'[' {
                start = Some(i);
                break;
            }
        }
        let start = start?;

        let mut end = None;
        for (i, b) in body.iter().copied().enumerate().rev() {
            if b == b'}' || b == b']' {
                end = Some(i);
                break;
            }
        }
        let end = end?;
        if end <= start {
            return None;
        }

        let slice = &body[start..=end];
        serde_json::from_slice(slice)
            .ok()
            .or_else(|| self.try_parse_backslash_escaped_json(slice))
            .or_else(|| self.try_parse_backslash_quote_json(slice))
    }

    fn trim_body<'a>(&self, body: &'a [u8]) -> &'a [u8] {
        let mut start = 0;
        while start < body.len() && (body[start].is_ascii_whitespace() || body[start] == 0) {
            start += 1;
        }
        let mut end = body.len();
        while end > start && (body[end - 1].is_ascii_whitespace() || body[end - 1] == 0) {
            end -= 1;
        }
        &body[start..end]
    }

    fn handle_request(
        &self,
        request: &mut tiny_http::Request,
    ) -> (Response<std::io::Cursor<Vec<u8>>>, bool) {
        const MAX_REQUEST_BODY: usize = 1024 * 1024;

        if request.method() != &tiny_http::Method::Post {
            return (
                self.error_response(Value::Null, -32600, "Invalid Request")
                    .with_status_code(405),
                false,
            );
        }

        let mut buf = Vec::new();
        match request.body_length() {
            Some(len) if len > 0 => {
                if len > MAX_REQUEST_BODY {
                    return (
                        self.error_response(Value::Null, -32600, "Request too large"),
                        false,
                    );
                }
                buf.resize(len, 0);
                if request.as_reader().read_exact(&mut buf).is_err() {
                    return (
                        self.error_response(Value::Null, -32700, "Parse error"),
                        false,
                    );
                }
            }
            _ => {
                let mut reader = request.as_reader().take((MAX_REQUEST_BODY + 1) as u64);
                if reader.read_to_end(&mut buf).is_err() {
                    return (
                        self.error_response(Value::Null, -32700, "Parse error"),
                        false,
                    );
                }
                if buf.len() > MAX_REQUEST_BODY {
                    return (
                        self.error_response(Value::Null, -32600, "Request too large"),
                        false,
                    );
                }
            }
        }

        if buf.is_empty() {
            return (
                self.error_response(Value::Null, -32700, "Parse error"),
                false,
            );
        }

        self.handle_json_rpc_body(&buf)
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

    fn sendrawtransaction(&self, params: &Value) -> Result<Value, String> {
        let hex_str = params
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or("Invalid hex")?;

        let raw = hex::decode(hex_str).map_err(|_| "Invalid hex".to_string())?;

        let (tx, _) = crate::consensus::transaction::Transaction::decode(&raw)
            .map_err(|_| "Failed to decode transaction".to_string())?;

        tx.check_structure()
            .map_err(|e| format!("Invalid transaction: {:?}", e))?;

        let txid = tx.txid();

        match self.mempool.add_transaction(tx) {
            Ok(_) => Ok(json!(hex::encode(txid))),
            Err(e) => Err(format!("Mempool rejected: {}", e)),
        }
    }

    fn getblockchaininfo(&self) -> Result<Value, String> {
        // Serve from cache if the entry is less than 1 second old.
        {
            let cache = self.blockchain_info_cache.lock().unwrap();
            if let Some((ts, ref v)) = *cache {
                if ts.elapsed() < Duration::from_secs(1) {
                    return Ok(v.clone());
                }
            }
        }

        let chain = self.chain.read().map_err(|_| "Lock poisoned".to_string())?;
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

        let v =
            serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))?;
        *self.blockchain_info_cache.lock().unwrap() = Some((Instant::now(), v.clone()));
        Ok(v)
    }

    fn getmininginfo(&self) -> Result<Value, String> {
        // Serve from cache if the entry is less than 1 second old.
        {
            let cache = self.mininginfo_cache.lock().unwrap();
            if let Some((ts, ref v)) = *cache {
                if ts.elapsed() < Duration::from_secs(1) {
                    return Ok(v.clone());
                }
            }
        }

        let chain = self.chain.read().map_err(|_| "Lock poisoned".to_string())?;
        let tip = chain.get_tip().map_err(|e| format!("{:?}", e))?;

        let blocks = tip.as_ref().map(|t| t.height as u32).unwrap_or(0);
        let bits = tip.as_ref().map(|t| t.block.header.n_bits).unwrap_or(0);

        let difficulty = difficulty_from_compact(bits).unwrap_or(0.0);
        let networkhashps = difficulty * 4294967296.0 / 150.0;

        let response = GetMiningInfoResponse {
            blocks,
            difficulty,
            networkhashps,
            chain: "ferrous".to_string(),
        };

        let v =
            serde_json::to_value(response).map_err(|e| format!("Serialization error: {}", e))?;
        *self.mininginfo_cache.lock().unwrap() = Some((Instant::now(), v.clone()));
        Ok(v)
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
                let mut chain = self
                    .chain
                    .write()
                    .map_err(|_| "Lock poisoned".to_string())?;
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
                    .write()
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
        let chain = self.chain.read().map_err(|_| "Lock poisoned".to_string())?;
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
        let chain = self.chain.read().map_err(|_| "Lock poisoned".to_string())?;
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
        let mut chain = self
            .chain
            .write()
            .map_err(|_| "Lock poisoned".to_string())?;

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

        let chain = self.chain.read().map_err(|_| "Lock poisoned".to_string())?;

        let block = match chain.get_block(&hash) {
            Some(b) => b.clone(),
            None => chain
                .block_store
                .get_block(&hash)
                .map_err(|e| e.to_string())?
                .ok_or("Block not found".to_string())?,
        };

        let height = chain
            .get_height_for_hash(&hash)
            .or_else(|| {
                chain
                    .block_store
                    .get_block_meta(&hash)
                    .ok()
                    .flatten()
                    .map(|m| m.height)
            })
            .unwrap_or(0) as u32;

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

        let chain = self.chain.read().map_err(|_| "Lock poisoned".to_string())?;

        let hash = chain
            .block_store
            .get_hash_by_height(height)
            .map_err(|e| e.to_string())?
            .ok_or("Block height out of range".to_string())?;

        Ok(json!(hex::encode(hash)))
    }

    fn getbestblockhash(&self) -> Result<Value, String> {
        let chain = self.chain.read().map_err(|_| "Lock poisoned".to_string())?;
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

    fn error_response(
        &self,
        id: Value,
        code: i32,
        message: &str,
    ) -> Response<std::io::Cursor<Vec<u8>>> {
        self.json_response(self.error_value(id, code, message))
    }

    fn json_response(&self, body: Value) -> Response<std::io::Cursor<Vec<u8>>> {
        Response::from_string(body.to_string()).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        )
    }

    fn success_value(&self, id: Value, result: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "result": result,
            "id": id
        })
    }

    fn error_value(&self, id: Value, code: i32, message: impl AsRef<str>) -> Value {
        json!({
            "jsonrpc": "2.0",
            "error": {
                "code": code,
                "message": message.as_ref()
            },
            "id": id
        })
    }
}

fn difficulty_from_compact(bits: u32) -> Option<f64> {
    if bits == 0 {
        return None;
    }
    let exponent = ((bits >> 24) & 0xff) as i32;
    let mantissa_u32 = bits & 0x00ff_ffff;
    if mantissa_u32 == 0 {
        return None;
    }

    let mantissa = mantissa_u32 as f64;
    let target = mantissa * 2f64.powi(8 * (exponent - 3));

    let diff1_mantissa = 0x0000ffffu32 as f64;
    let diff1_exponent = 0x1d_i32;
    let diff1_target = diff1_mantissa * 2f64.powi(8 * (diff1_exponent - 3));

    Some(diff1_target / target)
}
