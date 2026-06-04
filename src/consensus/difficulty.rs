use crate::consensus::block::{BlockHeader, TargetError, U256};
use crate::consensus::params::ChainParams;
use num_bigint::BigUint;

pub const MAINNET_TARGET_BLOCK_TIME: u64 = 150;

/// Trailing block intervals for the LWMA difficulty adjustment (Monero-style).
/// A single interval (or unweighted mean) regulates the median (~150/ln2 ≈ 216 s
/// mean for exponential PoW times); linearly weighting recent intervals makes
/// the controller mean-seeking at the 150 s target.
pub const DIFFICULTY_WINDOW: usize = 45;
/// Mainnet: RandomX conservative launch target — 10 leading zero bits.
/// Big-endian: 0x003FFFFF...FFFF (2^246 - 1). Easier than testnet at launch;
/// the ±1% per-block difficulty algorithm tightens toward the actual hashrate.
pub const MAINNET_MAX_TARGET: U256 = U256([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x3F, 0x00,
]);

/// Testnet: RandomX calibrated for 26 H/s (2 nodes × 13 H/s, 1 vCPU each,
/// FERROUS_MINER_THREADS=1) at 150 s block time.
/// target = floor(2^256 / (26 × 150)) = floor(2^256 / 3900).
/// Big-endian: 0x0010CDD9AA677344... — requires 11 leading zero bits.
/// compact(TESTNET_MAX_TARGET) = 0x1F10CDD9 (genesis_n_bits in params.rs).
pub const TESTNET_MAX_TARGET: U256 = U256([
    0x10, 0x40, 0x34, 0x77, 0xA6, 0x9A, 0xDD, 0x0C, 0x01, 0x44, 0x73, 0x67, 0xAA, 0xD9, 0xCD, 0x10,
    0x40, 0x34, 0x77, 0xA6, 0x9A, 0xDD, 0x0C, 0x01, 0x44, 0x73, 0x67, 0xAA, 0xD9, 0xCD, 0x10, 0x00,
]);

/// Regtest: near-trivial (n_bits 0x207fffff) — instant block generation for tests.
pub const REGTEST_MAX_TARGET: U256 = U256([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF,
    0xFF, 0x7F,
]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifficultyError {
    InvalidTimestamp,
    Overflow,
    TargetError(TargetError),
    TargetMismatch { expected: U256, actual: U256 },
}

pub fn u256_to_compact(target: &U256) -> u32 {
    let mut bytes = target.0;
    bytes.reverse();

    let mut index = 0usize;

    while index < bytes.len() && bytes[index] == 0 {
        index += 1;
    }

    if index == bytes.len() {
        return 0;
    }

    let len = (bytes.len() - index) as u8;
    let mut exponent = len;
    let mantissa: u32;

    if len <= 3 {
        let mut m = 0u32;

        for i in 0..len {
            m = (m << 8) | bytes[index + usize::from(i)] as u32;
        }

        m <<= 8 * (3 - len as u32);
        mantissa = m;
    } else {
        mantissa = (bytes[index] as u32) << 16
            | (bytes[index + 1] as u32) << 8
            | (bytes[index + 2] as u32);
    }

    let mut mantissa = mantissa;

    if mantissa & 0x0080_0000 != 0 {
        mantissa >>= 8;
        exponent = exponent.saturating_add(1);
    }

    (u32::from(exponent) << 24) | (mantissa & 0x00FF_FFFF)
}

/// `window_timestamps`: up to DIFFICULTY_WINDOW timestamps of the blocks
/// ending at `prev_header`, in chain order (oldest first). n timestamps span
/// n intervals through `current_timestamp`, combined as an LWMA (weight i for
/// the i-th oldest interval, newest weighted highest); fewer than 2 entries
/// falls back to the single prev→current interval.
pub fn calculate_next_target(
    prev_header: &BlockHeader,
    current_timestamp: u64,
    params: &ChainParams,
    window_timestamps: &[u64],
) -> Result<U256, DifficultyError> {
    let prev_target = prev_header
        .target()
        .map_err(|_| DifficultyError::InvalidTimestamp)?;

    if !params.difficulty_adjustment {
        return Ok(prev_target);
    }

    if params.allow_min_difficulty_blocks {
        let time_since_last = current_timestamp.saturating_sub(prev_header.timestamp);

        if time_since_last > params.target_block_time * 6 / 5 {
            return Ok(params.max_target);
        }
    }

    // Timestamps may legitimately decrease (block only needs to be > MTP, not > prev).
    // Per-interval saturating_sub: a backward step contributes a 0 interval; an
    // all-0 LWMA falls below min_timespan and gets clamped to target/4, producing
    // a 1% difficulty increase — correct Bitcoin-like behaviour.
    let mut actual_timespan = if window_timestamps.len() < 2 {
        current_timestamp.saturating_sub(prev_header.timestamp)
    } else {
        let n = window_timestamps.len() as u128;
        let mut weighted_sum: u128 = 0;
        let mut prev_ts = window_timestamps[0];
        for (i, &ts) in window_timestamps[1..]
            .iter()
            .chain(std::iter::once(&current_timestamp))
            .enumerate()
        {
            weighted_sum += (i as u128 + 1) * ts.saturating_sub(prev_ts) as u128;
            prev_ts = ts;
        }
        let weighted_count = n * (n + 1) / 2;
        (weighted_sum / weighted_count) as u64
    };

    let target = params.target_block_time;
    let min_timespan = target / 4;
    let max_timespan = target * 4;

    if actual_timespan < min_timespan {
        actual_timespan = min_timespan;
    } else if actual_timespan > max_timespan {
        actual_timespan = max_timespan;
    }

    let mut new_target =
        multiply_u256_by_factor(&prev_target, actual_timespan, target, &params.max_target)?;

    // Clamp adjustment to max ±1% per block
    let max_increase = multiply_u256_by_factor(&prev_target, 101, 100, &params.max_target)?;
    if new_target > max_increase {
        new_target = max_increase;
    }

    let max_decrease = multiply_u256_by_factor(&prev_target, 99, 100, &params.max_target)?;
    if new_target < max_decrease {
        new_target = max_decrease;
    }

    if new_target == U256([0u8; 32]) {
        let mut one_bytes = [0u8; 32];
        one_bytes[0] = 1;
        new_target = U256::from_le_bytes(one_bytes);
    }

    if new_target > params.max_target {
        new_target = params.max_target;
    }

    let n_bits = u256_to_compact(&new_target);

    let exponent = (n_bits >> 24) as u8;
    let mantissa = n_bits & 0x00FF_FFFF;

    let mut out_bytes = [0u8; 32];

    if exponent > 3 {
        let shift_bytes = usize::from(exponent - 3);

        if shift_bytes + 3 > 32 {
            return Err(DifficultyError::Overflow);
        }

        let mant_bytes = [
            (mantissa & 0xFF) as u8,
            ((mantissa >> 8) & 0xFF) as u8,
            ((mantissa >> 16) & 0xFF) as u8,
        ];

        for (i, b) in mant_bytes.iter().enumerate() {
            out_bytes[shift_bytes + i] = *b;
        }
    } else {
        let shift = 8 * (3_u32.saturating_sub(u32::from(exponent)));
        let value = mantissa >> shift;

        let mant_bytes = [
            (value & 0xFF) as u8,
            ((value >> 8) & 0xFF) as u8,
            ((value >> 16) & 0xFF) as u8,
        ];

        for (i, b) in mant_bytes.iter().enumerate() {
            if i < 32 {
                out_bytes[i] = *b;
            }
        }
    }

    Ok(U256::from_le_bytes(out_bytes))
}

fn multiply_u256_by_factor(
    target: &U256,
    factor: u64,
    scale: u64,
    max_target: &U256,
) -> Result<U256, DifficultyError> {
    let value = BigUint::from_bytes_le(&target.0);

    let factor_big = BigUint::from(factor);
    let scale_big = BigUint::from(scale);

    let result = (value * factor_big) / scale_big;

    let mut bytes = result.to_bytes_le();
    if bytes.len() > 32 {
        return Ok(*max_target);
    }
    bytes.resize(32, 0u8);

    Ok(U256::from_le_bytes(bytes.try_into().unwrap()))
}

pub fn validate_difficulty(
    prev_header: Option<&BlockHeader>,
    current_header: &BlockHeader,
    params: &ChainParams,
    window_timestamps: &[u64],
) -> Result<(), DifficultyError> {
    let Some(prev) = prev_header else {
        return Ok(());
    };

    let expected =
        calculate_next_target(prev, current_header.timestamp, params, window_timestamps)?;
    let actual = current_header
        .target()
        .map_err(|_| DifficultyError::InvalidTimestamp)?;

    if expected != actual {
        return Err(DifficultyError::TargetMismatch { expected, actual });
    }

    Ok(())
}
