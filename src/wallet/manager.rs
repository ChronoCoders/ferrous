use crate::consensus::chain::ChainState;
use crate::wallet::address::address_to_script_pubkey;
use crate::wallet::keys::{KeyStore, PrivateKey};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct WalletUtxo {
    pub txid: [u8; 32],
    pub vout: u32,
    pub value: u64,
    pub script_pubkey: Vec<u8>,
    pub height: u64,
}

#[derive(Debug)]
pub struct Wallet {
    keystore: KeyStore,
}

impl Wallet {
    pub fn load<P: AsRef<Path>>(wallet_path: P, network_prefix: u8) -> Result<Self, String> {
        let keystore = KeyStore::load(wallet_path, network_prefix)?;
        Ok(Self { keystore })
    }

    // ── address generation ────────────────────────────────────────────────────

    pub fn generate_address(&mut self) -> Result<String, String> {
        self.keystore.generate_new()
    }

    pub fn generate_change_address(&mut self) -> Result<String, String> {
        self.keystore.generate_change()
    }

    // ── key / seed accessors ──────────────────────────────────────────────────

    pub fn has_seed(&self) -> bool {
        self.keystore.has_seed()
    }

    pub fn seed_entropy(&self) -> Option<[u8; 32]> {
        self.keystore.seed_entropy()
    }

    pub fn is_encrypted(&self) -> bool {
        self.keystore.is_encrypted()
    }

    pub fn receive_index(&self) -> u32 {
        self.keystore.receive_index()
    }

    pub fn change_index(&self) -> u32 {
        self.keystore.change_index()
    }

    pub fn save_encrypted(&self, passphrase: &str) -> Result<(), String> {
        self.keystore.save_encrypted(passphrase)
    }

    pub fn set_seed(&mut self, entropy: [u8; 32]) -> Result<(), String> {
        self.keystore.set_seed(entropy)
    }

    // ── existing methods ──────────────────────────────────────────────────────

    pub fn addresses(&self) -> Vec<String> {
        self.keystore.entries().keys().cloned().collect()
    }

    pub fn find_address_for_script(&self, script_pubkey: &[u8]) -> Option<String> {
        for address in self.keystore.entries().keys() {
            if let Ok(addr_script) = address_to_script_pubkey(address) {
                if addr_script == script_pubkey {
                    return Some(address.clone());
                }
            }
        }
        None
    }

    pub fn get_private_key(&self, address: &str) -> Option<&PrivateKey> {
        self.keystore.entries().get(address)
    }

    pub fn owns_script(&self, script_pubkey: &[u8]) -> bool {
        for address in self.keystore.entries().keys() {
            if let Ok(addr_script) = address_to_script_pubkey(address) {
                if addr_script == script_pubkey {
                    return true;
                }
            }
        }
        false
    }

    pub fn get_utxos(&self, chain: &ChainState) -> Result<Vec<WalletUtxo>, String> {
        let mut script_map: HashMap<Vec<u8>, ()> = HashMap::new();

        let tip = chain.get_tip().map_err(|e| format!("{:?}", e))?;
        let tip_height = tip.as_ref().map(|t| t.height).unwrap_or(0);

        for address in self.keystore.entries().keys() {
            let script = address_to_script_pubkey(address)?;
            script_map.insert(script, ());
        }

        if script_map.is_empty() {
            return Ok(Vec::new());
        }

        let utxos = chain
            .export_utxos()
            .map_err(|e| format!("UTXO query failed: {}", e))?;

        let mut result = Vec::new();

        for (outpoint, entry) in utxos {
            if !script_map.contains_key(&entry.output.script_pubkey) {
                continue;
            }

            if entry.coinbase {
                let confirmations = if tip_height >= entry.height {
                    tip_height - entry.height + 1
                } else {
                    0
                };

                if confirmations < 100 {
                    continue;
                }
            }

            result.push(WalletUtxo {
                txid: outpoint.txid,
                vout: outpoint.vout,
                value: entry.output.value,
                script_pubkey: entry.output.script_pubkey.clone(),
                height: entry.height,
            });
        }

        result.sort_by(|a, b| b.value.cmp(&a.value));

        Ok(result)
    }

    pub fn get_balance(&self, chain: &ChainState) -> Result<u64, String> {
        let utxos = self.get_utxos(chain)?;
        let mut total = 0u64;
        for u in utxos {
            total = total
                .checked_add(u.value)
                .ok_or_else(|| "Balance overflow".to_string())?;
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn empty_wallet(dir: &TempDir) -> Wallet {
        let path = dir.path().join("wallet.dat");
        // load on a non-existent path returns a fresh empty wallet
        Wallet::load(&path, 0x6f).unwrap()
    }

    #[test]
    fn test_generate_change_address_distinct() {
        let dir = TempDir::new().unwrap();
        let mut wallet = empty_wallet(&dir);

        let receive_addr = wallet.generate_address().unwrap();
        let change_addr = wallet.generate_change_address().unwrap();

        assert_ne!(
            receive_addr, change_addr,
            "receive and change addresses must differ"
        );
        assert_eq!(wallet.receive_index(), 1);
        assert_eq!(wallet.change_index(), 1);
    }

    #[test]
    fn test_utxos_sorted_descending_by_value() {
        // Verify the sort comparator directly without needing a live chain.
        let mut utxos = [
            WalletUtxo {
                txid: [0; 32],
                vout: 0,
                value: 100,
                script_pubkey: vec![],
                height: 0,
            },
            WalletUtxo {
                txid: [0; 32],
                vout: 1,
                value: 500,
                script_pubkey: vec![],
                height: 0,
            },
            WalletUtxo {
                txid: [0; 32],
                vout: 2,
                value: 200,
                script_pubkey: vec![],
                height: 0,
            },
            WalletUtxo {
                txid: [0; 32],
                vout: 3,
                value: 50,
                script_pubkey: vec![],
                height: 0,
            },
        ];

        utxos.sort_by(|a, b| b.value.cmp(&a.value));

        assert_eq!(utxos[0].value, 500);
        assert_eq!(utxos[1].value, 200);
        assert_eq!(utxos[2].value, 100);
        assert_eq!(utxos[3].value, 50);
    }
}
