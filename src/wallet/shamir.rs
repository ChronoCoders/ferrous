use rand::RngCore;

// GF(256) arithmetic with irreducible polynomial x^8 + x^4 + x^3 + x + 1 (0x11b, AES field).
// Generator element: 0x03 (= x+1, primitive root of GF(256)* for this polynomial).
// Note: 0x02 has order 51 under this polynomial, NOT 255 — it is not a generator.

// Compile-time GF(256) multiply used only for building the EXP/LOG tables.
const fn gf_mul_const(a: u32, b: u32) -> u32 {
    let mut result = 0u32;
    let mut a = a;
    let mut b = b;
    let mut i = 0;
    while i < 8 {
        if b & 1 != 0 {
            result ^= a;
        }
        b >>= 1;
        let hi = a & 0x80;
        a = (a << 1) & 0xff;
        if hi != 0 {
            a ^= 0x1b; // reduce: x^8 ≡ x^4+x^3+x+1 = 0x1b mod 0x11b
        }
        i += 1;
    }
    result
}

const fn build_exp_table() -> [u8; 512] {
    let mut exp = [0u8; 512];
    let mut x: u32 = 1;
    let mut i: usize = 0;
    while i < 255 {
        exp[i] = x as u8;
        x = gf_mul_const(x, 3); // repeated multiplication by generator 0x03
        i += 1;
    }
    // Duplicate entries 0..254 into 255..509 so gf_mul needs no modulo.
    let mut i = 0;
    while i < 256 {
        exp[i + 255] = exp[i];
        i += 1;
    }
    exp
}

const fn build_log_table() -> [u8; 256] {
    let mut log = [0u8; 256];
    let mut x: u32 = 1;
    let mut i: usize = 0;
    while i < 255 {
        log[x as usize] = i as u8;
        x = gf_mul_const(x, 3);
        i += 1;
    }
    // log[0] is undefined; never accessed for nonzero inputs.
    log
}

static GF_EXP: [u8; 512] = build_exp_table();
static GF_LOG: [u8; 256] = build_log_table();

fn gf_mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    // log(a*b) = log(a) + log(b); both <= 254 so sum <= 508 < 512.
    GF_EXP[GF_LOG[a as usize] as usize + GF_LOG[b as usize] as usize]
}

fn gf_pow(base: u8, exp: u8) -> u8 {
    if exp == 0 {
        return 1;
    }
    if base == 0 {
        return 0;
    }
    // log(base^exp) = exp * log(base) mod 255.
    GF_EXP[(GF_LOG[base as usize] as usize * exp as usize) % 255]
}

fn gf_inv(a: u8) -> u8 {
    // a^254 = a^(-1) since a^255 = 1 in GF(256)*.
    gf_pow(a, 254)
}

fn eval_poly(coeffs: &[u8], x: u8) -> u8 {
    // Horner's method on coeffs = [c0, c1, ..., c(m-1)].
    // Iterating rev gives c(m-1), c(m-2), ..., c0.
    let mut result = 0u8;
    for &c in coeffs.iter().rev() {
        result = gf_mul(result, x) ^ c;
    }
    result
}

fn lagrange_at_zero(x: &[u8], y: &[u8]) -> u8 {
    // Evaluate the interpolating polynomial at 0.
    // L_i(0) = prod_{j≠i} x[j] / (x[i] XOR x[j])
    let mut result = 0u8;
    for i in 0..x.len() {
        let mut num = 1u8;
        let mut den = 1u8;
        for j in 0..x.len() {
            if i != j {
                num = gf_mul(num, x[j]);
                den = gf_mul(den, x[i] ^ x[j]);
            }
        }
        result ^= gf_mul(y[i], gf_mul(num, gf_inv(den)));
    }
    result
}

/// Split `secret` into `n` shares such that any `m` shares recover it.
///
/// Share format: `[index (1..=n), share_byte_0, share_byte_1, ...]`
/// Constraints: 2 ≤ m ≤ n ≤ 10, secret non-empty.
pub fn split(secret: &[u8], m: u8, n: u8) -> Result<Vec<Vec<u8>>, String> {
    if m < 2 || n < m || n > 10 {
        return Err(format!(
            "Invalid parameters: need 2 ≤ m ≤ n ≤ 10, got m={}, n={}",
            m, n
        ));
    }
    if secret.is_empty() {
        return Err("Secret must not be empty".to_string());
    }

    let mut rng = rand::thread_rng();
    let coeff_count = m as usize - 1;
    let mut rand_coeffs = vec![0u8; coeff_count];

    // Pre-allocate each share with its index byte.
    let mut shares: Vec<Vec<u8>> = (1u8..=n)
        .map(|i| {
            let mut v = Vec::with_capacity(1 + secret.len());
            v.push(i);
            v
        })
        .collect();

    let mut coeffs = Vec::with_capacity(m as usize);
    for &byte in secret {
        coeffs.clear();
        coeffs.push(byte);
        rng.fill_bytes(&mut rand_coeffs);
        coeffs.extend_from_slice(&rand_coeffs);

        for (idx, share) in shares.iter_mut().enumerate() {
            let x = (idx + 1) as u8;
            share.push(eval_poly(&coeffs, x));
        }
    }

    Ok(shares)
}

/// Recover the secret from `shares`. Requires at least `m` shares with consistent lengths.
///
/// Share format: `[index, share_byte_0, share_byte_1, ...]`
pub fn combine(shares: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    if shares.len() < 2 {
        return Err(format!("Need at least 2 shares, got {}", shares.len()));
    }
    let share_len = shares[0].len();
    if share_len < 2 {
        return Err("Each share must contain at least 2 bytes (index + data)".to_string());
    }
    for share in shares {
        if share.len() != share_len {
            return Err("All shares must have the same length".to_string());
        }
    }

    let x_coords: Vec<u8> = shares.iter().map(|s| s[0]).collect();
    for (i, &xi) in x_coords.iter().enumerate() {
        for &xj in &x_coords[i + 1..] {
            if xi == xj {
                return Err(format!("Duplicate share index: {}", xi));
            }
        }
    }

    let secret_len = share_len - 1;
    let mut secret = vec![0u8; secret_len];
    for byte_idx in 0..secret_len {
        let y_coords: Vec<u8> = shares.iter().map(|s| s[byte_idx + 1]).collect();
        secret[byte_idx] = lagrange_at_zero(&x_coords, &y_coords);
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_2_of_3_all_pairs() {
        let secret: Vec<u8> = (0u8..16).collect();
        let shares = split(&secret, 2, 3).unwrap();
        assert_eq!(shares.len(), 3);
        for (i, j) in [(0, 1), (0, 2), (1, 2)] {
            let recovered = combine(&[shares[i].clone(), shares[j].clone()]).unwrap();
            assert_eq!(recovered, secret, "pair ({i},{j}) failed");
        }
    }

    #[test]
    fn test_3_of_5_all_combinations() {
        let secret: Vec<u8> = (100u8..132).collect(); // 32 bytes
        let shares = split(&secret, 3, 5).unwrap();
        for i in 0..5 {
            for j in i + 1..5 {
                for k in j + 1..5 {
                    let recovered =
                        combine(&[shares[i].clone(), shares[j].clone(), shares[k].clone()])
                            .unwrap();
                    assert_eq!(recovered, secret, "combo ({i},{j},{k}) failed");
                }
            }
        }
    }

    #[test]
    fn test_threshold_enforcement() {
        let secret = vec![0x42u8; 16];

        // Fewer than 2 shares → error from combine.
        let shares = split(&secret, 2, 3).unwrap();
        assert!(combine(&shares[0..1]).is_err());

        // 2 shares from a 3-of-5 scheme produce a wrong (not original) secret.
        let shares5 = split(&secret, 3, 5).unwrap();
        let wrong = combine(&[shares5[0].clone(), shares5[1].clone()]).unwrap();
        assert_ne!(wrong, secret);
    }

    #[test]
    fn test_single_byte_secret() {
        let secret = vec![0xABu8];
        let shares = split(&secret, 2, 3).unwrap();
        // Use shares 0 and 2 (non-adjacent) to verify order independence.
        let recovered = combine(&[shares[0].clone(), shares[2].clone()]).unwrap();
        assert_eq!(recovered, secret);
    }

    #[test]
    fn test_invalid_params() {
        let s = vec![1u8, 2, 3];
        assert!(split(&s, 1, 3).is_err(), "m=1 must be rejected");
        assert!(split(&s, 4, 3).is_err(), "m>n must be rejected");
        assert!(split(&s, 2, 11).is_err(), "n>10 must be rejected");
        assert!(split(&[], 2, 3).is_err(), "empty secret must be rejected");

        // combine validations
        let shares = split(&s, 2, 3).unwrap();
        assert!(combine(&[]).is_err(), "0 shares must be rejected");
        assert!(combine(&shares[0..1]).is_err(), "1 share must be rejected");

        // Duplicate share indices
        assert!(
            combine(&[shares[0].clone(), shares[0].clone()]).is_err(),
            "duplicate index must be rejected"
        );
    }

    #[test]
    fn test_bip39_integration() {
        use crate::wallet::bip39::{entropy_to_mnemonic, generate_entropy, mnemonic_to_entropy};

        let entropy = generate_entropy(128).unwrap();
        let mnemonic = entropy_to_mnemonic(&entropy).unwrap();
        let entropy_rt = mnemonic_to_entropy(&mnemonic).unwrap();

        // Split the 16-byte entropy 2-of-3.
        let shares = split(&entropy_rt, 2, 3).unwrap();

        // Recover with shares 1 and 2 (indices 0-based).
        let recovered = combine(&[shares[1].clone(), shares[2].clone()]).unwrap();
        assert_eq!(recovered, entropy_rt);

        // Recovered bytes must produce the same valid mnemonic.
        let recovered_mnemonic = entropy_to_mnemonic(&recovered).unwrap();
        assert_eq!(recovered_mnemonic, mnemonic);
    }
}
