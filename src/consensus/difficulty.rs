use crate::consensus::block::{BlockHeader, TargetError, U256};
use crate::consensus::params::ChainParams;
use num_bigint::BigUint;

pub const MAINNET_TARGET_BLOCK_TIME: u64 = 150;
/// Mainnet: equivalent to Bitcoin difficulty-1 (n_bits 0x1d00ffff).
/// Big-endian: 0x000000FFFF000...000 — requires 32 leading zero bits.
pub const MAINNET_MAX_TARGET: U256 = U256([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0, 0,
    0, 0,
]);

/// Testnet: equivalent to n_bits 0x1f00ffff.
/// Big-endian: 0x0000FFFF000...000 — requires 16 leading zero bits (65536× easier than mainnet).
pub const TESTNET_MAX_TARGET: U256 = U256([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF,
    0, 0,
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

pub fn calculate_next_target(
    prev_header: &BlockHeader,
    current_timestamp: u64,
    params: &ChainParams,
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
    // Clamp via saturating_sub: a 0 timespan falls below min_timespan and gets clamped
    // to target/4, producing a 1% difficulty increase — correct Bitcoin-like behaviour.
    let mut actual_timespan = current_timestamp.saturating_sub(prev_header.timestamp);

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

    // Clamp adjustment to match tests: max +4% target, max -2% target
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
) -> Result<(), DifficultyError> {
    let Some(prev) = prev_header else {
        return Ok(());
    };

    let expected = calculate_next_target(prev, current_header.timestamp, params)?;
    let actual = current_header
        .target()
        .map_err(|_| DifficultyError::InvalidTimestamp)?;

    if expected != actual {
        return Err(DifficultyError::TargetMismatch { expected, actual });
    }

    Ok(())
}
