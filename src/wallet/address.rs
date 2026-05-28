use bech32::{Bech32m, Hrp};

const MAINNET_HRP: &str = "frr";
const TESTNET_HRP: &str = "tfrr";

/// Hash a Dilithium public key (1952 bytes) to a 32-byte address payload.
pub fn dilithium_pubkey_hash(pubkey_bytes: &[u8]) -> [u8; 32] {
    blake3::hash(pubkey_bytes).into()
}

fn hrp_for_prefix(network_prefix: u8) -> Result<Hrp, String> {
    match network_prefix {
        0x00 => Ok(Hrp::parse_unchecked(MAINNET_HRP)),
        0x6f => Ok(Hrp::parse_unchecked(TESTNET_HRP)),
        _ => Err(format!("Unknown network prefix: 0x{:02x}", network_prefix)),
    }
}

/// Encode a 32-byte hash as a bech32m address.
pub fn hash_to_address(hash: &[u8; 32], network_prefix: u8) -> String {
    let hrp = hrp_for_prefix(network_prefix).expect("invalid network prefix");
    bech32::encode::<Bech32m>(hrp, hash).expect("bech32m encode failed")
}

/// Decode a bech32m address to its 32-byte hash and network prefix byte.
pub fn address_to_hash(address: &str) -> Result<([u8; 32], u8), String> {
    let (hrp, data) =
        bech32::decode(address).map_err(|e| format!("Invalid bech32m address: {}", e))?;

    let prefix = match hrp.as_str() {
        MAINNET_HRP => 0x00u8,
        TESTNET_HRP => 0x6fu8,
        s => return Err(format!("Unknown HRP: {}", s)),
    };

    let hash: [u8; 32] = data
        .try_into()
        .map_err(|_| "Address payload must be exactly 32 bytes".to_string())?;

    Ok((hash, prefix))
}

/// Full pipeline: Dilithium pubkey bytes → bech32m address string.
pub fn pubkey_to_address(pubkey_bytes: &[u8], network_prefix: u8) -> String {
    let hash = dilithium_pubkey_hash(pubkey_bytes);
    hash_to_address(&hash, network_prefix)
}

/// Build P2DL scriptPubKey: OP_HASH256 <push 32> <32-byte-hash> OP_EQUALVERIFY OP_CHECKSIG
/// Total: 1 + 1 + 32 + 1 + 1 = 36 bytes
pub fn address_to_script_pubkey(address: &str) -> Result<Vec<u8>, String> {
    let (hash, _prefix) = address_to_hash(address)?;
    let mut script = Vec::with_capacity(36);
    script.push(0xaa); // OP_HASH256
    script.push(0x20); // push 32 bytes
    script.extend_from_slice(&hash);
    script.push(0x88); // OP_EQUALVERIFY
    script.push(0xac); // OP_CHECKSIG
    Ok(script)
}

/// Reverse: extract 32-byte hash from P2DL script, encode as address.
pub fn script_pubkey_to_address(script: &[u8], network_prefix: u8) -> Option<String> {
    if script.len() == 36
        && script[0] == 0xaa
        && script[1] == 0x20
        && script[34] == 0x88
        && script[35] == 0xac
    {
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&script[2..34]);
        Some(hash_to_address(&hash, network_prefix))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_pubkey() -> Vec<u8> {
        (0u8..=255).cycle().take(1952).collect()
    }

    #[test]
    fn test_roundtrip_pubkey_to_address_to_hash() {
        let pk = fake_pubkey();
        let addr = pubkey_to_address(&pk, 0x6f);
        let (hash, prefix) = address_to_hash(&addr).expect("decode failed");
        assert_eq!(prefix, 0x6f);
        assert_eq!(hash, dilithium_pubkey_hash(&pk));
    }

    #[test]
    fn test_mainnet_prefix() {
        let pk = fake_pubkey();
        let addr = pubkey_to_address(&pk, 0x00);
        assert!(addr.starts_with("frr1"), "expected frr1..., got {}", addr);
    }

    #[test]
    fn test_testnet_prefix() {
        let pk = fake_pubkey();
        let addr = pubkey_to_address(&pk, 0x6f);
        assert!(addr.starts_with("tfrr1"), "expected tfrr1..., got {}", addr);
    }

    #[test]
    fn test_address_to_script_pubkey_length_and_format() {
        let pk = fake_pubkey();
        let addr = pubkey_to_address(&pk, 0x6f);
        let script = address_to_script_pubkey(&addr).expect("script failed");
        assert_eq!(script.len(), 36);
        assert_eq!(script[0], 0xaa); // OP_HASH256
        assert_eq!(script[1], 0x20); // push 32
        assert_eq!(script[34], 0x88); // OP_EQUALVERIFY
        assert_eq!(script[35], 0xac); // OP_CHECKSIG
        let expected_hash = dilithium_pubkey_hash(&pk);
        assert_eq!(&script[2..34], &expected_hash);
    }

    #[test]
    fn test_script_pubkey_to_address_roundtrip() {
        let pk = fake_pubkey();
        let addr = pubkey_to_address(&pk, 0x6f);
        let script = address_to_script_pubkey(&addr).unwrap();
        let recovered = script_pubkey_to_address(&script, 0x6f).expect("recover failed");
        assert_eq!(recovered, addr);
    }
}
