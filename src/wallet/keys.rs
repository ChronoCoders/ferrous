use crate::consensus::transaction::BlindingFactor;
use crate::consensus::utxo::OutPoint;
use crate::wallet::address::pubkey_to_address;
use crate::wallet::bip39::{entropy_to_mnemonic, mnemonic_to_seed};
use crate::wallet::dilithium::DilithiumKeypair;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use curve25519_dalek_ng::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek_ng::ristretto::RistrettoPoint;
use curve25519_dalek_ng::scalar::Scalar;
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── wire format ──────────────────────────────────────────────────────────────
// [4]  magic  0x46455252 ("FERR")
// [1]  version 0x03  (0x02 = ECDSA-era Vec<[u8;32]>, 0x03 = Dilithium Vec<Vec<u8>>)
// [1]  flags  bit0=encrypted bit1=has_seed
// [32] salt   (KDF input, random)
// [12] nonce  (ChaCha20-Poly1305)
// [8]  payload_len  (LE u64, ciphertext bytes, excluding tag)
// [N]  ciphertext   (plaintext when not encrypted)
// [16] auth_tag     (Poly1305 tag; zeroed when not encrypted)

const MAGIC: [u8; 4] = [0x46, 0x45, 0x52, 0x52];
const VERSION: u8 = 0x03;
const FLAG_ENCRYPTED: u8 = 0x01;
const FLAG_HAS_SEED: u8 = 0x02;
const PBKDF2_ITERS: u32 = 210_000;
// header bytes before ciphertext: 4+1+1+32+12+8 = 58
const HEADER_LEN: usize = 58;

// Legacy ECDSA-era payload — keys stored as fixed [u8; 32].
// Used only for loading v0.02 wallets and migrating them to V3.
#[derive(Serialize, Deserialize)]
struct WalletPayloadV2 {
    seed_entropy: Option<[u8; 32]>,
    keys: Vec<(String, [u8; 32])>,
    receive_index: u32,
    change_index: u32,
}

// Current Dilithium-era payload — keys stored as Vec<u8> (32-byte seed).
#[derive(Serialize, Deserialize)]
struct WalletPayloadV3 {
    seed_entropy: Option<[u8; 32]>,
    keys: Vec<(String, Vec<u8>)>,
    receive_index: u32,
    change_index: u32,
}

// ── PrivateKey ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PrivateKey {
    inner: Vec<u8>, // Dilithium signing key seed — invariant: inner.len() == 32
}

impl PrivateKey {
    /// Constructs a `PrivateKey` from raw seed bytes. Returns `Err` if `inner.len() != 32`.
    pub fn new(inner: Vec<u8>) -> Result<Self, String> {
        if inner.len() != 32 {
            return Err(format!(
                "Invalid signing key length: {} (expected 32)",
                inner.len()
            ));
        }
        Ok(Self { inner })
    }

    pub fn key_bytes(&self) -> Vec<u8> {
        self.inner.clone()
    }

    pub fn public_key_bytes(&self) -> Vec<u8> {
        // Safe: inner.len() == 32 is guaranteed by PrivateKey::new.
        DilithiumKeypair::from_signing_key_bytes(&self.inner)
            .expect("32-byte inner is always a valid Dilithium seed")
            .verifying_key_bytes()
    }
}

// ── KeyStore ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct KeyStore {
    entries: HashMap<String, PrivateKey>,
    seed_entropy: Option<[u8; 32]>,
    receive_index: u32,
    change_index: u32,
    path: PathBuf,
    network_prefix: u8,
    encrypted: bool,
}

impl KeyStore {
    pub fn new<P: AsRef<Path>>(path: P, network_prefix: u8) -> Self {
        Self {
            entries: HashMap::new(),
            seed_entropy: None,
            receive_index: 0,
            change_index: 0,
            path: path.as_ref().to_path_buf(),
            network_prefix,
            encrypted: false,
        }
    }

    // ── load ─────────────────────────────────────────────────────────────────

    pub fn load<P: AsRef<Path>>(path: P, network_prefix: u8) -> Result<Self, String> {
        Self::load_inner(path.as_ref(), network_prefix, None)
    }

    pub fn load_encrypted<P: AsRef<Path>>(
        path: P,
        network_prefix: u8,
        passphrase: &str,
    ) -> Result<Self, String> {
        Self::load_inner(path.as_ref(), network_prefix, Some(passphrase))
    }

    fn load_inner(
        path: &Path,
        network_prefix: u8,
        passphrase: Option<&str>,
    ) -> Result<Self, String> {
        let path_buf = path.to_path_buf();

        if !path_buf.exists() {
            return Ok(Self::new(&path_buf, network_prefix));
        }

        let data = std::fs::read(&path_buf).map_err(|e| format!("Failed to read wallet: {}", e))?;

        // Detect format by magic bytes.
        if data.len() < 4 || data[..4] != MAGIC {
            return Self::migrate_csv(&data, path_buf, network_prefix);
        }

        if data.len() < HEADER_LEN + 16 {
            return Err("Wallet file is truncated".to_string());
        }

        let version = data[4];
        let flags = data[5];
        let is_encrypted = flags & FLAG_ENCRYPTED != 0;
        let salt = &data[6..38];
        let nonce_bytes = &data[38..50];
        let payload_len_bytes: [u8; 8] = data[50..58]
            .try_into()
            .map_err(|_| "Wallet header corrupt".to_string())?;
        let payload_len = u64::from_le_bytes(payload_len_bytes);
        if payload_len > usize::MAX as u64 {
            return Err("Wallet payload length overflow".to_string());
        }
        let payload_len = payload_len as usize;

        let ct_start = HEADER_LEN;
        let ct_end = ct_start + payload_len;
        let tag_end = ct_end + 16;

        if data.len() < tag_end {
            return Err("Wallet file is truncated".to_string());
        }

        let ciphertext = &data[ct_start..ct_end];
        let auth_tag = &data[ct_end..tag_end];

        let plaintext: Vec<u8> = if is_encrypted {
            let pass =
                passphrase.ok_or_else(|| "Wallet is encrypted; use load_encrypted".to_string())?;
            let mut key_bytes = [0u8; 32];
            pbkdf2_hmac::<Sha512>(pass.as_bytes(), salt, PBKDF2_ITERS, &mut key_bytes);

            let key = Key::from_slice(&key_bytes);
            let cipher = ChaCha20Poly1305::new(key);
            let nonce = Nonce::from_slice(nonce_bytes);

            let mut ct_with_tag = ciphertext.to_vec();
            ct_with_tag.extend_from_slice(auth_tag);

            cipher.decrypt(nonce, ct_with_tag.as_ref()).map_err(|_| {
                "Decryption failed: wrong passphrase or corrupted wallet".to_string()
            })?
        } else {
            ciphertext.to_vec()
        };

        match version {
            0x02 => {
                // ECDSA-era wallet: keys are [u8; 32]. Migrate to V3 on load.
                let wp: WalletPayloadV2 = bincode::deserialize(&plaintext)
                    .map_err(|e| format!("Wallet payload corrupt: {}", e))?;

                let mut entries = HashMap::with_capacity(wp.keys.len());
                for (addr, key_arr) in wp.keys {
                    let pk = PrivateKey::new(key_arr.to_vec())
                        .map_err(|e| format!("Invalid key in wallet: {}", e))?;
                    entries.insert(addr, pk);
                }

                let ks = Self {
                    entries,
                    seed_entropy: wp.seed_entropy,
                    receive_index: wp.receive_index,
                    change_index: wp.change_index,
                    path: path_buf,
                    network_prefix,
                    encrypted: is_encrypted,
                };

                // Re-save as V3 so future loads use the current format.
                ks.save_with_passphrase(passphrase)?;
                Ok(ks)
            }
            0x03 => {
                let wp: WalletPayloadV3 = bincode::deserialize(&plaintext)
                    .map_err(|e| format!("Wallet payload corrupt: {}", e))?;

                let mut entries = HashMap::with_capacity(wp.keys.len());
                for (addr, key_bytes) in wp.keys {
                    let pk = PrivateKey::new(key_bytes)
                        .map_err(|e| format!("Invalid key in wallet: {}", e))?;
                    entries.insert(addr, pk);
                }

                Ok(Self {
                    entries,
                    seed_entropy: wp.seed_entropy,
                    receive_index: wp.receive_index,
                    change_index: wp.change_index,
                    path: path_buf,
                    network_prefix,
                    encrypted: is_encrypted,
                })
            }
            v => Err(format!("Unknown wallet version: 0x{:02x}", v)),
        }
    }

    // ── save ──────────────────────────────────────────────────────────────────

    pub fn save(&self) -> Result<(), String> {
        self.save_with_passphrase(None)
    }

    pub fn save_encrypted(&self, passphrase: &str) -> Result<(), String> {
        self.save_with_passphrase(Some(passphrase))
    }

    fn save_with_passphrase(&self, passphrase: Option<&str>) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create wallet directory: {}", e))?;
        }

        let payload = WalletPayloadV3 {
            seed_entropy: self.seed_entropy,
            keys: self
                .entries
                .iter()
                .map(|(addr, pk)| (addr.clone(), pk.inner.clone()))
                .collect(),
            receive_index: self.receive_index,
            change_index: self.change_index,
        };

        let plaintext =
            bincode::serialize(&payload).map_err(|e| format!("Serialize wallet: {}", e))?;

        let mut rng = rand::thread_rng();
        let mut salt = [0u8; 32];
        let mut nonce_bytes = [0u8; 12];
        rng.fill_bytes(&mut salt);
        rng.fill_bytes(&mut nonce_bytes);

        let mut flags = 0u8;
        if self.seed_entropy.is_some() {
            flags |= FLAG_HAS_SEED;
        }

        let (ciphertext, auth_tag): (Vec<u8>, [u8; 16]) = if let Some(pass) = passphrase {
            flags |= FLAG_ENCRYPTED;
            let mut key_bytes = [0u8; 32];
            pbkdf2_hmac::<Sha512>(pass.as_bytes(), &salt, PBKDF2_ITERS, &mut key_bytes);

            let key = Key::from_slice(&key_bytes);
            let cipher = ChaCha20Poly1305::new(key);
            let nonce = Nonce::from_slice(&nonce_bytes);

            let ct_with_tag = cipher
                .encrypt(nonce, plaintext.as_ref())
                .map_err(|_| "Encryption failed".to_string())?;

            let tag_start = ct_with_tag.len() - 16;
            let mut tag = [0u8; 16];
            tag.copy_from_slice(&ct_with_tag[tag_start..]);
            (ct_with_tag[..tag_start].to_vec(), tag)
        } else {
            (plaintext, [0u8; 16])
        };

        let payload_len = ciphertext.len() as u64;

        let mut out = Vec::with_capacity(HEADER_LEN + ciphertext.len() + 16);
        out.extend_from_slice(&MAGIC);
        out.push(VERSION);
        out.push(flags);
        out.extend_from_slice(&salt);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&ciphertext);
        out.extend_from_slice(&auth_tag);

        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &out).map_err(|e| format!("Failed to write wallet tmp: {}", e))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("Failed to finalize wallet: {}", e))?;
        Ok(())
    }

    // ── key generation ────────────────────────────────────────────────────────

    pub fn generate_new(&mut self) -> Result<String, String> {
        let key = match self.seed_entropy {
            Some(entropy) => Self::derive_key(&entropy, b"receive", self.receive_index)?,
            None => {
                let kp = DilithiumKeypair::generate();
                PrivateKey::new(kp.signing_key_bytes())?
            }
        };
        let address = pubkey_to_address(&key.public_key_bytes(), self.network_prefix);
        self.entries.insert(address.clone(), key);
        self.receive_index += 1;
        self.save()?;
        Ok(address)
    }

    pub fn generate_change(&mut self) -> Result<String, String> {
        let key = match self.seed_entropy {
            Some(entropy) => Self::derive_key(&entropy, b"change", self.change_index)?,
            None => {
                let kp = DilithiumKeypair::generate();
                PrivateKey::new(kp.signing_key_bytes())?
            }
        };
        let address = pubkey_to_address(&key.public_key_bytes(), self.network_prefix);
        self.entries.insert(address.clone(), key);
        self.change_index += 1;
        self.save()?;
        Ok(address)
    }

    pub fn get_or_create_change(&mut self) -> Result<String, String> {
        if self.change_index == 0 {
            return self.generate_change();
        }
        match self.seed_entropy {
            Some(entropy) => {
                let pk = Self::derive_key(&entropy, b"change", 0)?;
                Ok(pubkey_to_address(
                    &pk.public_key_bytes(),
                    self.network_prefix,
                ))
            }
            None => self.generate_change(),
        }
    }

    // ── seed management ───────────────────────────────────────────────────────

    pub fn set_seed(&mut self, entropy: [u8; 32]) -> Result<(), String> {
        self.entries.clear();
        self.seed_entropy = Some(entropy);

        let rx = self.receive_index;
        let cx = self.change_index;
        for i in 0..rx {
            let pk = Self::derive_key(&entropy, b"receive", i)?;
            let addr = pubkey_to_address(&pk.public_key_bytes(), self.network_prefix);
            self.entries.insert(addr, pk);
        }
        for i in 0..cx {
            let pk = Self::derive_key(&entropy, b"change", i)?;
            let addr = pubkey_to_address(&pk.public_key_bytes(), self.network_prefix);
            self.entries.insert(addr, pk);
        }

        self.save()
    }

    pub fn has_seed(&self) -> bool {
        self.seed_entropy.is_some()
    }

    pub fn seed_entropy(&self) -> Option<[u8; 32]> {
        self.seed_entropy
    }

    pub fn bip39_seed(&self) -> Option<[u8; 64]> {
        let entropy = self.seed_entropy?;
        let mnemonic = entropy_to_mnemonic(&entropy).ok()?;
        Some(mnemonic_to_seed(&mnemonic, ""))
    }

    pub fn is_encrypted(&self) -> bool {
        self.encrypted
    }

    // ── accessors ─────────────────────────────────────────────────────────────

    pub fn entries(&self) -> &HashMap<String, PrivateKey> {
        &self.entries
    }

    pub fn receive_index(&self) -> u32 {
        self.receive_index
    }

    pub fn change_index(&self) -> u32 {
        self.change_index
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    fn derive_key(entropy: &[u8; 32], kind: &[u8], index: u32) -> Result<PrivateKey, String> {
        let mnemonic = entropy_to_mnemonic(entropy).map_err(|e| format!("BIP39: {}", e))?;
        let seed = mnemonic_to_seed(&mnemonic, "");
        let mut hasher = Sha512::new();
        hasher.update(seed);
        hasher.update(kind);
        hasher.update(index.to_le_bytes());
        let hash = hasher.finalize();
        let kp = DilithiumKeypair::from_signing_key_bytes(&hash[..32])
            .map_err(|e| format!("Derived key invalid (index {}): {}", index, e))?;
        PrivateKey::new(kp.signing_key_bytes())
            .map_err(|e| format!("Derived key invalid (index {}): {}", index, e))
    }

    // ── CSV migration ─────────────────────────────────────────────────────────

    fn migrate_csv(data: &[u8], path: PathBuf, network_prefix: u8) -> Result<Self, String> {
        let text = std::str::from_utf8(data)
            .map_err(|_| "Legacy wallet is not valid UTF-8".to_string())?;

        let mut entries = HashMap::new();
        for line in text.lines() {
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

            let raw = hex::decode(hex_key)
                .map_err(|e| format!("Invalid key hex in legacy wallet: {}", e))?;
            let key_bytes = csv_deobfuscate(&raw, address.as_bytes());
            let pk = PrivateKey::new(key_bytes)
                .map_err(|e| format!("Invalid key in legacy wallet: {}", e))?;
            entries.insert(address, pk);
        }

        let n = entries.len() as u32;
        let ks = Self {
            entries,
            seed_entropy: None,
            receive_index: n,
            change_index: 0,
            path,
            network_prefix,
            encrypted: false,
        };

        ks.save()
            .map_err(|e| format!("Legacy wallet migration failed: {}", e))?;
        Ok(ks)
    }
}

// XOR-based obfuscation used by the old CSV format.
fn csv_deobfuscate(obfuscated: &[u8], salt: &[u8]) -> Vec<u8> {
    let mask = Sha256::digest(salt);
    obfuscated
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ mask[i % 32])
        .collect()
}

fn scalar_from_blake3(label: &[u8], parts: &[&[u8]]) -> Scalar {
    let mut hasher = blake3::Hasher::new();
    hasher.update(label);
    for p in parts {
        hasher.update(p);
    }
    let mut reader = hasher.finalize_xof();
    let mut wide = [0u8; 64];
    reader.fill(&mut wide);
    Scalar::from_bytes_mod_order_wide(&wide)
}

pub fn derive_blinding(seed: &[u8; 64], outpoint: &OutPoint) -> BlindingFactor {
    let scalar = scalar_from_blake3(
        b"Ferrous/blinding",
        &[seed, &outpoint.txid, &outpoint.vout.to_le_bytes()],
    );
    BlindingFactor(scalar.to_bytes())
}

pub fn derive_view_key(seed: &[u8; 64]) -> (Scalar, RistrettoPoint) {
    let scalar = scalar_from_blake3(b"Ferrous/view", &[seed]);
    let point = scalar * RISTRETTO_BASEPOINT_POINT;
    (scalar, point)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_wallet(dir: &TempDir) -> PathBuf {
        dir.path().join("wallet.dat")
    }

    // ── 1. round-trip: no seed, no encryption ─────────────────────────────────
    #[test]
    fn test_save_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = tmp_wallet(&dir);

        let mut ks = KeyStore::new(&path, 0x6f);
        let addr = ks.generate_new().unwrap();
        assert!(ks.entries().contains_key(&addr));

        let ks2 = KeyStore::load(&path, 0x6f).unwrap();
        assert!(ks2.entries().contains_key(&addr));
        assert!(!ks2.is_encrypted());
        assert!(!ks2.has_seed());
        assert_eq!(ks2.receive_index, 1);
        assert_eq!(ks2.change_index, 0);
    }

    // ── 2. save/load with seed entropy ────────────────────────────────────────
    #[test]
    fn test_seed_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = tmp_wallet(&dir);

        let entropy = [0xABu8; 32];
        let mut ks = KeyStore::new(&path, 0x6f);
        ks.set_seed(entropy).unwrap();

        let addr = ks.generate_new().unwrap();

        let ks2 = KeyStore::load(&path, 0x6f).unwrap();
        assert_eq!(ks2.seed_entropy(), Some(entropy));
        assert!(ks2.has_seed());
        assert!(ks2.entries().contains_key(&addr));
    }

    // ── 3. encrypt → load with correct passphrase → keys intact ───────────────
    #[test]
    fn test_encrypt_decrypt_correct_passphrase() {
        let dir = TempDir::new().unwrap();
        let path = tmp_wallet(&dir);

        let mut ks = KeyStore::new(&path, 0x6f);
        let addr = ks.generate_new().unwrap();
        ks.save_encrypted("hunter2").unwrap();

        let ks2 = KeyStore::load_encrypted(&path, 0x6f, "hunter2").unwrap();
        assert!(ks2.entries().contains_key(&addr));
        assert!(ks2.is_encrypted());
    }

    // ── 4. load with wrong passphrase → Err ──────────────────────────────────
    #[test]
    fn test_wrong_passphrase_rejected() {
        let dir = TempDir::new().unwrap();
        let path = tmp_wallet(&dir);

        let mut ks = KeyStore::new(&path, 0x6f);
        ks.generate_new().unwrap();
        ks.save_encrypted("correct").unwrap();

        let result = KeyStore::load_encrypted(&path, 0x6f, "wrong");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Decryption failed") || msg.contains("wrong passphrase"),
            "unexpected: {}",
            msg
        );
    }

    // ── 5. old CSV format migration ───────────────────────────────────────────
    #[test]
    fn test_csv_migration() {
        let dir = TempDir::new().unwrap();
        let path = tmp_wallet(&dir);

        {
            let address = "mTestAddr1111111111111111111111111";
            let key_bytes = [0x01u8; 32];
            let mask = Sha256::digest(address.as_bytes());
            let obf: Vec<u8> = key_bytes
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ mask[i % 32])
                .collect();
            let csv = format!("{},{}\n", address, hex::encode(&obf));
            std::fs::write(&path, csv.as_bytes()).unwrap();
        }

        let ks = KeyStore::load(&path, 0x6f).unwrap();
        assert_eq!(ks.entries().len(), 1);
        assert_eq!(ks.receive_index, 1);
        assert!(!ks.is_encrypted());

        let data = std::fs::read(&path).unwrap();
        assert_eq!(&data[..4], &MAGIC);
        assert_eq!(data[4], VERSION); // must be v3 after migration
    }

    // ── 6. FERR v2 binary wallet migration ────────────────────────────────────
    #[test]
    fn test_v2_wallet_migration() {
        use bincode;

        let dir = TempDir::new().unwrap();
        let path = tmp_wallet(&dir);

        // Manually serialize a V2 payload and write a FERR v2 wallet file.
        let v2_key: [u8; 32] = [0x42u8; 32];
        let addr = "tfrr1fake_address_for_v2_migration_test";
        let wp_v2 = WalletPayloadV2 {
            seed_entropy: None,
            keys: vec![(addr.to_string(), v2_key)],
            receive_index: 1,
            change_index: 0,
        };

        let plaintext = bincode::serialize(&wp_v2).unwrap();
        let payload_len = plaintext.len() as u64;

        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.push(0x02); // version v2
        out.push(0x00); // flags: unencrypted, no seed
        out.extend_from_slice(&[0u8; 32]); // salt (unused for unencrypted)
        out.extend_from_slice(&[0u8; 12]); // nonce (unused for unencrypted)
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&plaintext); // ciphertext = plaintext
        out.extend_from_slice(&[0u8; 16]); // auth_tag (zeroed for unencrypted)
        std::fs::write(&path, &out).unwrap();

        // Load must migrate to v3 transparently.
        let ks = KeyStore::load(&path, 0x6f).unwrap();
        assert_eq!(ks.entries().len(), 1);
        assert!(ks.entries().contains_key(addr));
        assert_eq!(ks.receive_index, 1);
        assert_eq!(ks.change_index, 0);

        // Verify the key bytes were preserved (v2_key used as Dilithium seed).
        let pk = ks.entries().get(addr).unwrap();
        assert_eq!(pk.key_bytes(), v2_key.to_vec());

        // Disk file must now be v3.
        let data = std::fs::read(&path).unwrap();
        assert_eq!(&data[..4], &MAGIC);
        assert_eq!(data[4], VERSION); // 0x03
    }

    // ── 7. PrivateKey::new rejects wrong-length input ─────────────────────────
    #[test]
    fn test_private_key_length_validation() {
        assert!(PrivateKey::new(vec![0u8; 32]).is_ok());
        assert!(PrivateKey::new(vec![0u8; 31]).is_err());
        assert!(PrivateKey::new(vec![0u8; 33]).is_err());
        assert!(PrivateKey::new(vec![]).is_err());
    }

    // ── 8. generate_new increments receive_index; generate_change increments change_index
    #[test]
    fn test_index_increments() {
        let dir = TempDir::new().unwrap();
        let path = tmp_wallet(&dir);

        let mut ks = KeyStore::new(&path, 0x6f);
        assert_eq!(ks.receive_index, 0);
        assert_eq!(ks.change_index, 0);

        ks.generate_new().unwrap();
        assert_eq!(ks.receive_index, 1);
        assert_eq!(ks.change_index, 0);

        ks.generate_change().unwrap();
        assert_eq!(ks.receive_index, 1);
        assert_eq!(ks.change_index, 1);

        ks.generate_new().unwrap();
        assert_eq!(ks.receive_index, 2);
        assert_eq!(ks.change_index, 1);

        let ks2 = KeyStore::load(&path, 0x6f).unwrap();
        assert_eq!(ks2.receive_index, 2);
        assert_eq!(ks2.change_index, 1);
        assert_eq!(ks2.entries().len(), 3);
    }

    #[test]
    fn test_blinding_derivation_deterministic() {
        let seed = [0x33u8; 64];
        let op = OutPoint {
            txid: [1u8; 32],
            vout: 0,
        };

        let a = derive_blinding(&seed, &op);
        let b = derive_blinding(&seed, &op);
        assert_eq!(a, b, "same seed + outpoint must derive the same blinding");

        let op2 = OutPoint {
            txid: [1u8; 32],
            vout: 1,
        };
        let c = derive_blinding(&seed, &op2);
        assert_ne!(a, c, "different outpoint must derive a different blinding");

        let seed2 = [0x44u8; 64];
        let d = derive_blinding(&seed2, &op);
        assert_ne!(a, d, "different seed must derive a different blinding");
    }
}
