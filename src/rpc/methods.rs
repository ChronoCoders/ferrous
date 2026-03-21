use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlockchainInfoResponse {
    pub chain: String,
    pub blocks: u32,
    pub headers: u32,
    pub bestblockhash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MineBlocksRequest {
    pub nblocks: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MineBlocksResponse {
    pub blocks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlockRequest {
    pub blockhash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlockResponse {
    pub hash: String,
    pub height: u32,
    pub version: u32,
    pub merkleroot: String,
    pub time: u64,
    pub nonce: u64,
    pub bits: String,
    pub size: usize,
    pub n_tx: usize,
    pub miner: Option<String>,
    pub tx: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transactions: Option<Vec<VerboseTx>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerboseTxInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub txid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vout: Option<u32>,
    pub coinbase: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerboseTxOutput {
    pub value_frr: f64,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerboseTx {
    pub txid: String,
    pub is_coinbase: bool,
    pub vin: Vec<VerboseTxInput>,
    pub vout: Vec<VerboseTxOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetNewAddressResponse {
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBalanceResponse {
    pub balance: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListUnspentItem {
    pub txid: String,
    pub vout: u32,
    pub amount: f64,
    pub confirmations: u32,
    pub script_pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListUnspentResponse {
    pub utxos: Vec<ListUnspentItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendToAddressResponse {
    pub txid: String,
    pub blockhash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListAddressesResponse {
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMiningInfoResponse {
    pub blocks: u32,
    pub difficulty: f64,
    pub networkhashps: f64,
    pub hashrate: f64,
    pub chain: String,
}
