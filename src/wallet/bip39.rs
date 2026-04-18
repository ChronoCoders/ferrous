use crate::wallet::bip39_wordlist::WORDLIST;
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::{Digest, Sha256, Sha512};

/// Generate cryptographically random entropy of the given bit size.
/// Valid sizes: 128, 160, 192, 224, 256.
pub fn generate_entropy(bit_size: usize) -> Result<Vec<u8>, String> {
    match bit_size {
        128 | 160 | 192 | 224 | 256 => {}
        _ => {
            return Err(format!(
                "Invalid entropy size: {}. Must be 128/160/192/224/256.",
                bit_size
            ))
        }
    }
    let byte_len = bit_size / 8;
    let mut entropy = vec![0u8; byte_len];
    rand::thread_rng().fill_bytes(&mut entropy);
    Ok(entropy)
}

/// Convert raw entropy bytes to a BIP39 mnemonic string.
/// Appends checksum bits (entropy_bits / 32), groups into 11-bit chunks, maps to words.
pub fn entropy_to_mnemonic(entropy: &[u8]) -> Result<String, String> {
    let bit_len = entropy.len() * 8;
    match bit_len {
        128 | 160 | 192 | 224 | 256 => {}
        _ => return Err(format!("Invalid entropy length: {} bytes", entropy.len())),
    }

    let checksum_bits = bit_len / 32;
    let hash = Sha256::digest(entropy);
    // Extract top checksum_bits from the hash byte
    let checksum_byte = hash[0];

    // Build a bit stream: entropy bits followed by checksum bits
    let total_bits = bit_len + checksum_bits;
    let word_count = total_bits / 11;

    let mut words = Vec::with_capacity(word_count);
    for word_idx in 0..word_count {
        let bit_start = word_idx * 11;
        let mut val: u16 = 0;
        for bit_offset in 0..11 {
            let bit_pos = bit_start + bit_offset;
            let bit = if bit_pos < bit_len {
                // Entropy bit
                let byte_idx = bit_pos / 8;
                let bit_in_byte = 7 - (bit_pos % 8);
                (entropy[byte_idx] >> bit_in_byte) & 1
            } else {
                // Checksum bit — always read from MSB of hash[0]: bit 7, 6, 5, ...
                let cs_bit = bit_pos - bit_len;
                (checksum_byte >> (7 - cs_bit)) & 1
            };
            val = (val << 1) | (bit as u16);
        }
        words.push(WORDLIST[val as usize]);
    }

    Ok(words.join(" "))
}

/// Validate a mnemonic string and convert back to entropy bytes.
/// Returns Err if any word is not in the wordlist or checksum fails.
pub fn mnemonic_to_entropy(mnemonic: &str) -> Result<Vec<u8>, String> {
    // Normalize: collapse whitespace and lowercase
    let normalized: String = mnemonic
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    let words: Vec<&str> = normalized.split(' ').collect();
    let word_count = words.len();
    match word_count {
        12 | 15 | 18 | 21 | 24 => {}
        _ => return Err(format!("Invalid mnemonic word count: {}", word_count)),
    }

    // Map each word to its 11-bit index
    let mut indices = Vec::with_capacity(word_count);
    for word in &words {
        let idx = WORDLIST
            .iter()
            .position(|&w| w == *word)
            .ok_or_else(|| format!("Unknown word in mnemonic: '{}'", word))?;
        indices.push(idx as u16);
    }

    // Reconstruct the bit stream
    let total_bits = word_count * 11;
    let checksum_bits = total_bits / 33; // total_bits = entropy_bits + entropy_bits/32 = entropy_bits * 33/32
    let entropy_bits = total_bits - checksum_bits;
    let entropy_bytes = entropy_bits / 8;

    let mut bits: Vec<u8> = Vec::with_capacity(total_bits);
    for idx in &indices {
        for i in (0..11).rev() {
            bits.push((idx >> i) as u8 & 1);
        }
    }

    // Reconstruct entropy bytes
    let mut entropy = vec![0u8; entropy_bytes];
    for (i, byte) in entropy.iter_mut().enumerate() {
        let mut val: u8 = 0;
        for bit in 0..8 {
            val = (val << 1) | bits[i * 8 + bit];
        }
        *byte = val;
    }

    // Verify checksum
    let hash = Sha256::digest(&entropy);
    let checksum_byte = hash[0];
    for i in 0..checksum_bits {
        let expected_bit = (checksum_byte >> (7 - i)) & 1;
        let actual_bit = bits[entropy_bits + i];
        if expected_bit != actual_bit {
            return Err("Invalid mnemonic: checksum verification failed".to_string());
        }
    }

    Ok(entropy)
}

/// Derive a 64-byte seed from a mnemonic and optional passphrase.
/// PBKDF2-HMAC-SHA512, 2048 iterations, salt = "mnemonic" || passphrase.
pub fn mnemonic_to_seed(mnemonic: &str, passphrase: &str) -> [u8; 64] {
    let password = mnemonic.as_bytes();
    let salt = format!("mnemonic{}", passphrase);
    let mut seed = [0u8; 64];
    pbkdf2_hmac::<Sha512>(password, salt.as_bytes(), 2048, &mut seed);
    seed
}

#[cfg(test)]
mod tests {
    use super::*;

    // BIP39 test vector 0: 128-bit all-zeros entropy, passphrase "TREZOR"
    #[test]
    fn test_vector_128bit_zeros() {
        let entropy = hex::decode("00000000000000000000000000000000").unwrap();
        let mnemonic = entropy_to_mnemonic(&entropy).unwrap();
        assert_eq!(
            mnemonic,
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        );
        let seed = mnemonic_to_seed(&mnemonic, "TREZOR");
        let expected = hex::decode(
            "c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e53495531f09a6987599d18264c1e1c92f2cf141630c7a3c4ab7c81b2f001698e7463b04"
        ).unwrap();
        assert_eq!(seed.as_slice(), expected.as_slice());
    }

    // BIP39 test vector 11: 256-bit all-ones entropy, passphrase "TREZOR"
    #[test]
    fn test_vector_256bit_ones() {
        let entropy =
            hex::decode("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
                .unwrap();
        let mnemonic = entropy_to_mnemonic(&entropy).unwrap();
        assert_eq!(
            mnemonic,
            "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote"
        );
        let seed = mnemonic_to_seed(&mnemonic, "TREZOR");
        let expected = hex::decode(
            "dd48c104698c30cfe2b6142103248622fb7bb0ff692eebb00089b32d22484e1613912f0a5b694407be899ffd31ed3992c456cdf60f5d4564b8ba3f05a69890ad"
        ).unwrap();
        assert_eq!(seed.as_slice(), expected.as_slice());
    }

    // Round-trip: generate_entropy → entropy_to_mnemonic → mnemonic_to_entropy
    #[test]
    fn test_round_trip_128() {
        let entropy = generate_entropy(128).unwrap();
        let mnemonic = entropy_to_mnemonic(&entropy).unwrap();
        let recovered = mnemonic_to_entropy(&mnemonic).unwrap();
        assert_eq!(entropy, recovered);
    }

    // Invalid checksum is rejected
    #[test]
    fn test_invalid_checksum_rejected() {
        // Valid mnemonic with last word changed to corrupt checksum
        let result = mnemonic_to_entropy(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon zoo",
        );
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("checksum") || msg.contains("Unknown word"),
            "unexpected error: {}",
            msg
        );
    }

    // Unknown word is rejected
    #[test]
    fn test_invalid_word_rejected() {
        let result = mnemonic_to_entropy(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon notaword",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown word"));
    }
}
