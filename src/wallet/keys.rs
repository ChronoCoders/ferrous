use crate::wallet::address::pubkey_to_address;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PrivateKey {
    inner: SecretKey,
}

impl PrivateKey {
    pub fn new(inner: SecretKey) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &SecretKey {
        &self.inner
    }

    pub fn public_key_bytes(&self) -> Vec<u8> {
        let secp = Secp256k1::new();
        let pubkey = PublicKey::from_secret_key(&secp, &self.inner);
        pubkey.serialize().to_vec()
    }
}

#[derive(Debug, Clone)]
pub struct KeyStore {
    entries: HashMap<String, PrivateKey>,
    path: PathBuf,
    network_prefix: u8,
}

impl KeyStore {
    pub fn new<P: AsRef<Path>>(path: P, network_prefix: u8) -> Self {
        Self {
            entries: HashMap::new(),
            path: path.as_ref().to_path_buf(),
            network_prefix,
        }
    }

    pub fn load<P: AsRef<Path>>(path: P, network_prefix: u8) -> Result<Self, String> {
        let path_buf = path.as_ref().to_path_buf();

        if !path_buf.exists() {
            return Ok(Self::new(path_buf, network_prefix));
        }

        let file = File::open(&path_buf).map_err(|e| format!("Failed to open wallet: {}", e))?;
        let reader = BufReader::new(file);

        let mut entries = HashMap::new();

        for line in reader.lines() {
            let line = line.map_err(|e| format!("Failed to read wallet: {}", e))?;
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.split(',');
            let address = parts
                .next()
                .ok_or_else(|| "Malformed wallet line: missing address".to_string())?
                .to_string();
            let hex_key = parts
                .next()
                .ok_or_else(|| "Malformed wallet line: missing key".to_string())?;

            let key_bytes = hex::decode(hex_key)
                .map_err(|e| format!("Invalid key hex in wallet.dat: {}", e))?;

            let secret_key = SecretKey::from_slice(&key_bytes)
                .map_err(|e| format!("Invalid secret key in wallet.dat: {}", e))?;

            entries.insert(address, PrivateKey::new(secret_key));
        }

        Ok(Self {
            entries,
            path: path_buf,
            network_prefix,
        })
    }

    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create wallet directory: {}", e))?;
        }

        let mut file =
            File::create(&self.path).map_err(|e| format!("Failed to create wallet: {}", e))?;

        for (address, key) in &self.entries {
            let key_hex = hex::encode(key.inner.secret_bytes());
            let line = format!("{},{}\n", address, key_hex);
            file.write_all(line.as_bytes())
                .map_err(|e| format!("Failed to write wallet: {}", e))?;
        }

        Ok(())
    }

    pub fn entries(&self) -> &HashMap<String, PrivateKey> {
        &self.entries
    }

    pub fn entries_mut(&mut self) -> &mut HashMap<String, PrivateKey> {
        &mut self.entries
    }

    pub fn generate_new(&mut self) -> Result<String, String> {
        let mut rng = rand::thread_rng();
        let secret_key = SecretKey::new(&mut rng);
        let key = PrivateKey::new(secret_key);
        let pubkey = key.public_key_bytes();
        let address = pubkey_to_address(&pubkey, self.network_prefix);
        self.entries.insert(address.clone(), key);
        self.save()?;
        Ok(address)
    }
}
