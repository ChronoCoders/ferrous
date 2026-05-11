use ml_dsa::{
    Generate, KeyExport, KeyInit, Keypair, MlDsa65, Signature, SigningKey, Signer, VerifyingKey,
    Verifier,
};

pub struct DilithiumKeypair {
    signing_key: SigningKey<MlDsa65>,
}

impl DilithiumKeypair {
    pub fn generate() -> Self {
        Self {
            signing_key: SigningKey::<MlDsa65>::generate(),
        }
    }

    /// Returns the 32-byte seed that encodes the signing key for storage.
    pub fn signing_key_bytes(&self) -> Vec<u8> {
        let seed = self.signing_key.to_bytes();
        seed.iter().copied().collect()
    }

    /// Returns the 1952-byte ML-DSA-65 verifying (public) key.
    pub fn verifying_key_bytes(&self) -> Vec<u8> {
        let vk = self.signing_key.verifying_key();
        let encoded = vk.to_bytes();
        encoded.iter().copied().collect()
    }

    /// Signs `msg` and returns the 3309-byte ML-DSA-65 signature.
    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        let sig: Signature<MlDsa65> = self.signing_key.sign(msg);
        let encoded = sig.encode();
        encoded.iter().copied().collect()
    }

    /// Reconstructs a keypair from the 32-byte seed returned by `signing_key_bytes`.
    pub fn from_signing_key_bytes(bytes: &[u8]) -> Result<Self, String> {
        SigningKey::<MlDsa65>::new_from_slice(bytes)
            .map(|sk| Self { signing_key: sk })
            .map_err(|e| format!("Invalid Dilithium signing key: {}", e))
    }
}

/// Verifies an ML-DSA-65 signature.
pub fn verify(pubkey_bytes: &[u8], msg: &[u8], sig_bytes: &[u8]) -> Result<(), String> {
    let pk = VerifyingKey::<MlDsa65>::new_from_slice(pubkey_bytes)
        .map_err(|e| format!("Invalid Dilithium public key: {}", e))?;

    let sig = Signature::<MlDsa65>::try_from(sig_bytes)
        .map_err(|e| format!("Invalid Dilithium signature: {}", e))?;

    pk.verify(msg, &sig)
        .map_err(|e| format!("Dilithium verification failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_sign_verify_roundtrip() {
        let kp = DilithiumKeypair::generate();
        let msg = b"ferrous dilithium test";
        let sig = kp.sign(msg);
        let pk_bytes = kp.verifying_key_bytes();
        assert_eq!(pk_bytes.len(), 1952);
        assert_eq!(sig.len(), 3309);
        verify(&pk_bytes, msg, &sig).expect("roundtrip verify failed");
    }

    #[test]
    fn test_wrong_message_verify_fails() {
        let kp = DilithiumKeypair::generate();
        let sig = kp.sign(b"correct message");
        let pk_bytes = kp.verifying_key_bytes();
        let result = verify(&pk_bytes, b"wrong message", &sig);
        assert!(result.is_err(), "verify should fail for wrong message");
    }

    #[test]
    fn test_wrong_pubkey_verify_fails() {
        let kp1 = DilithiumKeypair::generate();
        let kp2 = DilithiumKeypair::generate();
        let msg = b"ferrous dilithium test";
        let sig = kp1.sign(msg);
        let wrong_pk = kp2.verifying_key_bytes();
        let result = verify(&wrong_pk, msg, &sig);
        assert!(result.is_err(), "verify should fail for wrong pubkey");
    }

    #[test]
    fn test_roundtrip_from_signing_key_bytes() {
        let kp1 = DilithiumKeypair::generate();
        let sk_bytes = kp1.signing_key_bytes();
        let kp2 = DilithiumKeypair::from_signing_key_bytes(&sk_bytes)
            .expect("from_signing_key_bytes failed");
        assert_eq!(kp1.verifying_key_bytes(), kp2.verifying_key_bytes());
    }
}
