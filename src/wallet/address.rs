use crate::primitives::hash::sha256d;
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    let ripemd = Ripemd160::digest(sha);
    let mut out = [0u8; 20];
    out.copy_from_slice(&ripemd);
    out
}

pub fn pubkey_to_address(pubkey: &[u8], network_prefix: u8) -> String {
    let mut payload = Vec::with_capacity(1 + 20);
    payload.push(network_prefix);
    let h160 = hash160(pubkey);
    payload.extend_from_slice(&h160);

    let checksum = sha256d(&payload);
    let mut data = payload;
    data.extend_from_slice(&checksum[0..4]);

    bs58::encode(data).into_string()
}

/// Decode a P2PKH script_pubkey back to a Base58Check address.
/// Returns None if the script is not a standard P2PKH (76 a9 14 <20 bytes> 88 ac).
pub fn script_pubkey_to_address(script: &[u8], network_prefix: u8) -> Option<String> {
    if script.len() == 25
        && script[0] == 0x76
        && script[1] == 0xa9
        && script[2] == 0x14
        && script[23] == 0x88
        && script[24] == 0xac
    {
        let hash160 = &script[3..23];
        let mut payload = Vec::with_capacity(21);
        payload.push(network_prefix);
        payload.extend_from_slice(hash160);
        let checksum = sha256d(&payload);
        let mut data = payload;
        data.extend_from_slice(&checksum[0..4]);
        Some(bs58::encode(data).into_string())
    } else {
        None
    }
}

pub fn address_to_script_pubkey(address: &str) -> Result<Vec<u8>, String> {
    let data = bs58::decode(address)
        .into_vec()
        .map_err(|e| format!("Invalid Base58 address: {}", e))?;

    if data.len() < 25 {
        return Err("Address too short".to_string());
    }

    let (payload, checksum) = data.split_at(data.len() - 4);
    let expected = sha256d(payload);
    if expected[0..4] != checksum[..] {
        return Err("Invalid address checksum".to_string());
    }

    if payload.len() != 21 {
        return Err("Invalid payload length".to_string());
    }

    let network_prefix = payload[0];
    if network_prefix != 0x6f {
        return Err("Invalid network prefix".to_string());
    }

    let hash160 = &payload[1..21];

    let mut script = Vec::with_capacity(25);
    script.push(0x76);
    script.push(0xa9);
    script.push(0x14);
    script.extend_from_slice(hash160);
    script.push(0x88);
    script.push(0xac);

    Ok(script)
}
