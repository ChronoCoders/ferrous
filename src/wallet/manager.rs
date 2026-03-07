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

    pub fn generate_address(&mut self) -> Result<String, String> {
        self.keystore.generate_new()
    }

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
