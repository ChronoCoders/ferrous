use ferrous_node::consensus::block::BlockHeader;
use ferrous_node::consensus::chain::ChainState;
use ferrous_node::consensus::merkle::compute_merkle_root;
use ferrous_node::consensus::params::Network;
use ferrous_node::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
use ferrous_node::mining::Miner;
use ferrous_node::primitives::hash::Hash256;
use ferrous_node::rpc::RpcServer;
use ferrous_node::wallet::address::address_to_script_pubkey;
use ferrous_node::wallet::builder::TransactionBuilder;
use ferrous_node::wallet::manager::Wallet;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn zero_hash() -> Hash256 {
    [0u8; 32]
}

fn sample_output(value: u64) -> TxOutput {
    TxOutput {
        value,
        script_pubkey: vec![0x51],
    }
}

fn empty_witnesses(input_count: usize) -> Vec<Witness> {
    let mut v = Vec::with_capacity(input_count);
    for _ in 0..input_count {
        v.push(Witness {
            stack_items: Vec::new(),
        });
    }
    v
}

fn coinbase_transaction(value: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig: Vec::new(),
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![sample_output(value)],
        witnesses: empty_witnesses(1),
        locktime: 0,
    }
}

fn create_chain() -> (ChainState, String, u32, TempDir) {
    let tx = coinbase_transaction(50 * 100_000_000);
    let txids = vec![tx.txid()];
    let merkle_root = compute_merkle_root(&txids);

    let mut header = BlockHeader {
        version: 1,
        prev_block_hash: zero_hash(),
        merkle_root,
        timestamp: 1_000_000_000,
        n_bits: 0x207f_ffff,
        nonce: 0,
    };

    while !header.check_proof_of_work().unwrap() {
        header.nonce += 1;
    }

    let best_hash_hex = hex::encode(header.hash());

    let temp_dir = TempDir::new().unwrap();
    let chain = ChainState::new(
        Network::Regtest.params(),
        temp_dir.path().to_str().unwrap(),
        Some((header, tx)),
    )
    .unwrap();

    (chain, best_hash_hex, 0, temp_dir)
}

fn read_body(response: tiny_http::Response<std::io::Cursor<Vec<u8>>>) -> Value {
    let mut body = String::new();
    response
        .into_reader()
        .read_to_string(&mut body)
        .expect("read body failed");
    serde_json::from_str(&body).expect("invalid json body")
}

#[test]
fn test_rpc_server_creation() {
    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0");
    assert!(server.is_ok());
}

#[test]
fn test_getblockchaininfo_returns_correct_data() {
    let (chain, expected_hash, expected_height, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let request = json!({
        "jsonrpc": "2.0",
        "method": "getblockchaininfo",
        "params": [],
        "id": 1
    });

    let response = server.handle_json_rpc(request);
    let body = read_body(response);

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);

    let result = &body["result"];
    assert_eq!(result["chain"], "ferrous");
    assert_eq!(result["blocks"], expected_height);
    assert_eq!(result["headers"], expected_height);
    assert_eq!(result["bestblockhash"], expected_hash);
}

#[test]
fn test_unknown_method_returns_error() {
    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let request = json!({
        "jsonrpc": "2.0",
        "method": "unknown_method",
        "params": [],
        "id": 2
    });

    let response = server.handle_json_rpc(request);
    let body = read_body(response);

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 2);
    assert!(body.get("error").is_some());
    assert_eq!(body["error"]["code"], -32601);
}

#[test]
fn test_json_rpc_error_handling_parse_error() {
    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let response = server.handle_raw("this is not valid json");
    let body = read_body(response);

    assert_eq!(body["jsonrpc"], "2.0");
    assert!(body.get("error").is_some());
    assert_eq!(body["error"]["code"], -32700);
}

#[test]
fn test_mineblocks_mines_correct_count_and_returns_hashes() {
    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let request = json!({
        "jsonrpc": "2.0",
        "method": "mineblocks",
        "params": [3],
        "id": 10
    });

    let response = server.handle_json_rpc(request);
    let body = read_body(response);

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 10);

    let result = &body["result"];
    let blocks = result["blocks"].as_array().expect("blocks array");
    assert_eq!(blocks.len(), 3);
    for h in blocks {
        let s = h.as_str().expect("hash string");
        assert_eq!(s.len(), 64);
    }
}

#[test]
fn test_mineblocks_extends_chain_height() {
    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain.clone(), miner, wallet, "127.0.0.1:0").expect("server");

    let info_before = server.handle_json_rpc(json!({
        "jsonrpc": "2.0",
        "method": "getblockchaininfo",
        "params": [],
        "id": 20
    }));
    let body_before = read_body(info_before);
    let height_before = body_before["result"]["blocks"].as_u64().unwrap();

    let mine_request = json!({
        "jsonrpc": "2.0",
        "method": "mineblocks",
        "params": [5],
        "id": 21
    });
    let _ = read_body(server.handle_json_rpc(mine_request));

    let info_after = server.handle_json_rpc(json!({
        "jsonrpc": "2.0",
        "method": "getblockchaininfo",
        "params": [],
        "id": 22
    }));
    let body_after = read_body(info_after);
    let height_after = body_after["result"]["blocks"].as_u64().unwrap();

    assert_eq!(height_after, height_before + 5);
}

#[test]
fn test_mineblocks_zero_blocks_returns_error() {
    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let request = json!({
        "jsonrpc": "2.0",
        "method": "mineblocks",
        "params": [0],
        "id": 11
    });

    let response = server.handle_json_rpc(request);
    let body = read_body(response);

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 11);
    assert!(body.get("error").is_some());
}

#[test]
fn test_mineblocks_too_many_blocks_returns_error() {
    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let request = json!({
        "jsonrpc": "2.0",
        "method": "mineblocks",
        "params": [1001],
        "id": 12
    });

    let response = server.handle_json_rpc(request);
    let body = read_body(response);

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 12);
    assert!(body.get("error").is_some());
}

#[test]
fn test_getblock_returns_correct_data() {
    let (mut chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    // mine one block to have something retrievable
    let header = miner
        .mine_and_attach(&mut chain, Vec::new())
        .expect("mine block");
    let hash_hex = hex::encode(header.hash());

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let request = json!({
        "jsonrpc": "2.0",
        "method": "getblock",
        "params": [hash_hex],
        "id": 13
    });

    let response = server.handle_json_rpc(request);
    let body = read_body(response);

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 13);

    let result = &body["result"];
    assert_eq!(result["hash"].as_str().unwrap().len(), 64);
    assert!(result["height"].as_u64().is_some());
    assert!(result["version"].as_u64().is_some());
    assert_eq!(result["merkleroot"].as_str().unwrap().len(), 64);
    assert!(result["time"].as_u64().is_some());
    assert!(result["nonce"].as_u64().is_some());
    assert!(result["bits"].as_str().unwrap().len() == 8);
    let tx = result["tx"].as_array().unwrap();
    assert!(!tx.is_empty());
}

#[test]
fn test_getblock_invalid_hash_returns_error() {
    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let request = json!({
        "jsonrpc": "2.0",
        "method": "getblock",
        "params": ["zzzz"],
        "id": 14
    });

    let response = server.handle_json_rpc(request);
    let body = read_body(response);

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 14);
    assert!(body.get("error").is_some());
}

#[test]
fn test_getbestblockhash_returns_tip_hash() {
    let (mut chain, tip_hash, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    // mine one block so tip changes
    let header = miner
        .mine_and_attach(&mut chain, Vec::new())
        .expect("mine block");
    let new_tip_hash = hex::encode(header.hash());
    assert_ne!(new_tip_hash, tip_hash);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let request = json!({
        "jsonrpc": "2.0",
        "method": "getbestblockhash",
        "params": [],
        "id": 15
    });

    let response = server.handle_json_rpc(request);
    let body = read_body(response);

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 15);
    let result = &body["result"];
    let hash_str = result.as_str().unwrap();
    assert_eq!(hash_str.len(), 64);
    assert_eq!(hash_str, &new_tip_hash);
}

#[test]
fn test_generatetoaddress_mines_to_wallet_address() {
    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let addr_req = json!({
        "jsonrpc": "2.0",
        "method": "getnewaddress",
        "params": [],
        "id": 100
    });
    let addr_body = read_body(server.handle_json_rpc(addr_req));
    let address = addr_body["result"]["address"]
        .as_str()
        .expect("address string")
        .to_string();

    let gen_req = json!({
        "jsonrpc": "2.0",
        "method": "generatetoaddress",
        "params": [101, address],
        "id": 101
    });
    let gen_body = read_body(server.handle_json_rpc(gen_req));
    let blocks = gen_body["result"]["blocks"]
        .as_array()
        .expect("blocks array");
    assert_eq!(blocks.len(), 101);

    let bal_req = json!({
        "jsonrpc": "2.0",
        "method": "getbalance",
        "params": [],
        "id": 102
    });
    let bal_body = read_body(server.handle_json_rpc(bal_req));
    let balance = bal_body["result"]["balance"]
        .as_f64()
        .expect("balance number");
    assert!(balance > 0.0);
}

#[test]
fn test_sendtoaddress_end_to_end() {
    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let chain_for_recipient = chain.clone();
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));
    let wallet_for_inspection = wallet.clone();

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let addr_req = json!({
        "jsonrpc": "2.0",
        "method": "getnewaddress",
        "params": [],
        "id": 200
    });
    let addr_body = read_body(server.handle_json_rpc(addr_req));
    let funding_address = addr_body["result"]["address"]
        .as_str()
        .expect("address string")
        .to_string();

    let gen_req = json!({
        "jsonrpc": "2.0",
        "method": "generatetoaddress",
        "params": [101, funding_address],
        "id": 201
    });
    let _ = read_body(server.handle_json_rpc(gen_req));

    let bal_before_req = json!({
        "jsonrpc": "2.0",
        "method": "getbalance",
        "params": [],
        "id": 202
    });
    let bal_before_body = read_body(server.handle_json_rpc(bal_before_req));
    let balance_before = bal_before_body["result"]["balance"]
        .as_f64()
        .expect("balance number");
    assert!(balance_before > 0.0);

    let recipient_tmp = TempDir::new().unwrap();
    let recipient_wallet_path = recipient_tmp.path().join("recipient-wallet.dat");
    let mut recipient_wallet = Wallet::load(&recipient_wallet_path, 0x6f).unwrap();
    let recipient_address = recipient_wallet.generate_address().unwrap();

    let amount_btc = 1.0f64;
    let send_req = json!({
        "jsonrpc": "2.0",
        "method": "sendtoaddress",
        "params": [recipient_address, amount_btc],
        "id": 203
    });
    let send_body = read_body(server.handle_json_rpc(send_req));

    assert_eq!(send_body["jsonrpc"], "2.0");
    assert_eq!(send_body["id"], 203);

    let result = &send_body["result"];
    let txid = result["txid"].as_str().expect("txid string").to_string();
    let blockhash = result["blockhash"]
        .as_str()
        .expect("blockhash string")
        .to_string();
    assert_eq!(txid.len(), 64);
    assert_eq!(blockhash.len(), 64);

    let getblock_req = json!({
        "jsonrpc": "2.0",
        "method": "getblock",
        "params": [blockhash.clone()],
        "id": 204
    });
    let getblock_body = read_body(server.handle_json_rpc(getblock_req));
    assert_eq!(getblock_body["jsonrpc"], "2.0");
    assert_eq!(getblock_body["id"], 204);
    let block_result = &getblock_body["result"];
    let tx_array = block_result["tx"].as_array().expect("tx array");
    let mut found_tx = false;
    for v in tx_array {
        if v.as_str().expect("txid") == txid {
            found_tx = true;
            break;
        }
    }
    assert!(found_tx);

    let bal_after_req = json!({
        "jsonrpc": "2.0",
        "method": "getbalance",
        "params": [],
        "id": 205
    });
    let bal_after_body = read_body(server.handle_json_rpc(bal_after_req));
    let balance_after = bal_after_body["result"]["balance"]
        .as_f64()
        .expect("balance number");
    assert!(balance_after > 0.0);

    let amount_sats = (amount_btc * 100_000_000f64).round() as u64;
    let fee_sats = 1000u64;

    let chain_guard = chain_for_recipient.lock().unwrap();

    let blockhash_bytes = hex::decode(blockhash).expect("valid blockhash hex");
    assert_eq!(blockhash_bytes.len(), 32);
    let mut blockhash_arr = [0u8; 32];
    blockhash_arr.copy_from_slice(&blockhash_bytes);

    let block = chain_guard
        .get_block(&blockhash_arr)
        .expect("block query")
        .expect("block present");

    let tx = block
        .transactions
        .iter()
        .find(|t| hex::encode(t.txid()) == txid)
        .expect("tx in block");

    let dest_script = address_to_script_pubkey(&recipient_address).unwrap();

    assert_eq!(tx.outputs.len(), 2);

    let dest_output = tx
        .outputs
        .iter()
        .find(|o| o.script_pubkey == dest_script)
        .expect("dest output");
    assert_eq!(dest_output.value, amount_sats);

    let wallet_guard = wallet_for_inspection.lock().unwrap();
    let wallet_scripts: Vec<Vec<u8>> = wallet_guard
        .addresses()
        .into_iter()
        .map(|addr| address_to_script_pubkey(&addr).unwrap())
        .collect();
    drop(wallet_guard);

    let change_output = tx
        .outputs
        .iter()
        .find(|o| wallet_scripts.contains(&o.script_pubkey))
        .expect("change output");

    let tip_height = chain_guard.get_tip().expect("tip").height;
    let mut input_sum = 0u64;
    for input in &tx.inputs {
        let mut found_prev = false;
        for h in 0..=tip_height {
            if let Some(prev_block) = chain_guard.get_block_at_height(h).expect("block at height") {
                for prev_tx in &prev_block.transactions {
                    if prev_tx.txid() == input.prev_txid {
                        let prev_out = &prev_tx.outputs[input.prev_index as usize];
                        input_sum = input_sum
                            .checked_add(prev_out.value)
                            .expect("input sum overflow");
                        found_prev = true;
                        break;
                    }
                }
                if found_prev {
                    break;
                }
            }
        }
        assert!(found_prev);
    }

    let expected_change = input_sum - amount_sats - fee_sats;
    assert_eq!(change_output.value, expected_change);

    let recipient_balance_sats = recipient_wallet.get_balance(&chain_guard).unwrap();
    assert_eq!(recipient_balance_sats, amount_sats);
}

#[test]
fn test_insufficient_funds_rejection() {
    let (chain, _, _, tmp) = create_chain();
    let wallet_path = tmp.path().join("insufficient-wallet.dat");
    let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();
    let recipient_address = wallet.generate_address().unwrap();

    let amount_sats = 1_000_000_000_000u64;
    let fee_sats = 1000u64;

    let result = TransactionBuilder::create_transaction(
        &wallet,
        &chain,
        &recipient_address,
        amount_sats,
        fee_sats,
    );
    assert!(matches!(result, Err(ref e) if e == "Insufficient funds"));

    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let addr_req = json!({
        "jsonrpc": "2.0",
        "method": "getnewaddress",
        "params": [],
        "id": 300
    });
    let addr_body = read_body(server.handle_json_rpc(addr_req));
    let funding_address = addr_body["result"]["address"]
        .as_str()
        .expect("address string")
        .to_string();

    let gen_req = json!({
        "jsonrpc": "2.0",
        "method": "generatetoaddress",
        "params": [101, funding_address],
        "id": 301
    });
    let _ = read_body(server.handle_json_rpc(gen_req));

    let bal_req = json!({
        "jsonrpc": "2.0",
        "method": "getbalance",
        "params": [],
        "id": 302
    });
    let bal_body = read_body(server.handle_json_rpc(bal_req));
    let balance = bal_body["result"]["balance"]
        .as_f64()
        .expect("balance number");
    assert!(balance > 0.0);

    let recipient_tmp = TempDir::new().unwrap();
    let recipient_wallet_path = recipient_tmp.path().join("recipient-wallet.dat");
    let mut recipient_wallet = Wallet::load(&recipient_wallet_path, 0x6f).unwrap();
    let recipient_address_rpc = recipient_wallet.generate_address().unwrap();

    let amount_btc = balance + 1.0;
    let send_req = json!({
        "jsonrpc": "2.0",
        "method": "sendtoaddress",
        "params": [recipient_address_rpc, amount_btc],
        "id": 303
    });
    let send_body = read_body(server.handle_json_rpc(send_req));

    assert_eq!(send_body["jsonrpc"], "2.0");
    assert_eq!(send_body["id"], 303);
    assert!(send_body.get("error").is_some());
    assert_eq!(send_body["error"]["code"], -32603);
    let message = send_body["error"]["message"]
        .as_str()
        .expect("error message");
    assert!(
        message.contains("Transaction creation failed: Insufficient funds"),
        "unexpected error message: {}",
        message
    );
}

#[test]
fn test_invalid_address_rejection() {
    let malformed = "not_base58!!";
    let err = address_to_script_pubkey(malformed).unwrap_err();
    assert!(
        err.starts_with("Invalid Base58 address:"),
        "unexpected malformed error: {}",
        err
    );

    let tmp_mainnet = TempDir::new().unwrap();
    let mainnet_wallet_path = tmp_mainnet.path().join("mainnet-wallet.dat");
    let mut mainnet_wallet = Wallet::load(&mainnet_wallet_path, 0x00).unwrap();
    let mainnet_address = mainnet_wallet.generate_address().unwrap();
    let err = address_to_script_pubkey(&mainnet_address).unwrap_err();
    assert_eq!(err, "Invalid network prefix");

    let tmp_regtest = TempDir::new().unwrap();
    let reg_wallet_path = tmp_regtest.path().join("reg-wallet.dat");
    let mut reg_wallet = Wallet::load(&reg_wallet_path, 0x6f).unwrap();
    let reg_address = reg_wallet.generate_address().unwrap();
    let mut data = bs58::decode(&reg_address).into_vec().unwrap();
    let last = data.len() - 1;
    data[last] ^= 0x01;
    let bad_checksum_address = bs58::encode(data).into_string();
    let err = address_to_script_pubkey(&bad_checksum_address).unwrap_err();
    assert_eq!(err, "Invalid address checksum");

    let (chain, _, _, _tmp) = create_chain();
    let miner = Miner::new(Network::Regtest.params(), vec![0x51]);

    let chain = Arc::new(Mutex::new(chain));
    let miner = Arc::new(miner);
    let wallet = Arc::new(Mutex::new(Wallet::load("./test-wallet.dat", 0x6f).unwrap()));

    let server = RpcServer::new(chain, miner, wallet, "127.0.0.1:0").expect("server");

    let invalid_addresses = vec![malformed.to_string(), mainnet_address, bad_checksum_address];

    for (i, addr) in invalid_addresses.into_iter().enumerate() {
        let send_req = json!({
            "jsonrpc": "2.0",
            "method": "sendtoaddress",
            "params": [addr, 1.0f64],
            "id": 400 + i as i32
        });
        let send_body = read_body(server.handle_json_rpc(send_req));

        assert_eq!(send_body["jsonrpc"], "2.0");
        assert_eq!(send_body["id"], 400 + i as i32);
        assert!(send_body.get("error").is_some());
        assert_eq!(send_body["error"]["code"], -32603);

        let message = send_body["error"]["message"]
            .as_str()
            .expect("error message");
        assert!(
            message.starts_with("Transaction creation failed:"),
            "unexpected RPC error message: {}",
            message
        );
    }
}
