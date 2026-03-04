use ferrous_node::consensus::block::{BlockHeader, U256};
use ferrous_node::consensus::difficulty::{
    calculate_next_target, u256_to_compact, validate_difficulty, DifficultyError,
};
use ferrous_node::consensus::params::{ChainParams, Network};
use ferrous_node::primitives::hash::Hash256;

fn zero_hash() -> Hash256 {
    [0u8; 32]
}

fn header_with_target_and_timestamp(n_bits: u32, timestamp: u64) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_block_hash: zero_hash(),
        merkle_root: zero_hash(),
        timestamp,
        n_bits,
        nonce: 0,
    }
}

fn u256_to_u128(value: &U256) -> u128 {
    let mut out = 0u128;

    for (i, byte) in value.0.iter().take(16).enumerate() {
        out |= (*byte as u128) << (8 * i);
    }

    out
}

const BASE_NBITS: u32 = 0x0401_0000;

fn mainnet_params() -> ChainParams {
    Network::Mainnet.params()
}

fn testnet_params() -> ChainParams {
    Network::Testnet.params()
}

fn regtest_params() -> ChainParams {
    Network::Regtest.params()
}

#[test]
fn compact_roundtrip_consistency() {
    let nbits_values = vec![0x207f_ffff, BASE_NBITS, 0x0100_1234, 0x2000_0001];

    for n in nbits_values {
        let header = header_with_target_and_timestamp(n, 1_234_567);
        let target = header.target().unwrap();

        let compact = u256_to_compact(&target);
        let roundtrip = header_with_target_and_timestamp(compact, 1_234_567);
        let decoded = roundtrip.target().unwrap();

        assert_eq!(decoded, target);
    }
}

#[test]
fn target_increases_when_delta_above_target() {
    let prev = header_with_target_and_timestamp(BASE_NBITS, 1_000);
    let prev_value = u256_to_u128(&prev.target().unwrap());

    let next = calculate_next_target(&prev, 1_300, &mainnet_params()).unwrap();
    let next_value = u256_to_u128(&next);

    assert!(next_value > prev_value);
}

#[test]
fn target_decreases_when_delta_below_target() {
    let prev = header_with_target_and_timestamp(BASE_NBITS, 1_000);
    let prev_value = u256_to_u128(&prev.target().unwrap());

    let next = calculate_next_target(&prev, 1_060, &mainnet_params()).unwrap();
    let next_value = u256_to_u128(&next);

    assert!(next_value < prev_value);
}

#[test]
fn delta_is_clamped_to_minimum() {
    let prev = header_with_target_and_timestamp(BASE_NBITS, 1_000);

    let next_small_delta = calculate_next_target(&prev, 1_010, &mainnet_params()).unwrap();
    let next_min_delta = calculate_next_target(&prev, 1_030, &mainnet_params()).unwrap();

    assert_eq!(
        u256_to_u128(&next_small_delta),
        u256_to_u128(&next_min_delta)
    );
}

#[test]
fn delta_is_clamped_to_maximum() {
    let prev = header_with_target_and_timestamp(BASE_NBITS, 1_000);

    let next_large_delta = calculate_next_target(&prev, 2_000, &mainnet_params()).unwrap();
    let next_max_delta = calculate_next_target(&prev, 1_600, &mainnet_params()).unwrap();

    assert_eq!(
        u256_to_u128(&next_large_delta),
        u256_to_u128(&next_max_delta)
    );
}

#[test]
fn max_increase_factor_is_enforced() {
    let prev = header_with_target_and_timestamp(BASE_NBITS, 1_000);
    let prev_value = u256_to_u128(&prev.target().unwrap());

    let next = calculate_next_target(&prev, 1_600, &mainnet_params()).unwrap();
    let next_value = u256_to_u128(&next);

    let max_allowed = prev_value * 104 / 100;

    assert!(next_value >= prev_value);
    assert!(next_value <= max_allowed);
}

#[test]
fn max_decrease_factor_is_enforced() {
    let prev = header_with_target_and_timestamp(BASE_NBITS, 1_000);
    let prev_value = u256_to_u128(&prev.target().unwrap());

    let next = calculate_next_target(&prev, 1_030, &mainnet_params()).unwrap();
    let next_value = u256_to_u128(&next);

    let min_allowed = prev_value * 98 / 100;

    assert!(next_value <= prev_value);
    assert!(next_value >= min_allowed);
}

#[test]
fn delta_equal_to_target_produces_no_change() {
    let prev = header_with_target_and_timestamp(BASE_NBITS, 1_000);
    let prev_value = u256_to_u128(&prev.target().unwrap());

    let next = calculate_next_target(&prev, 1_150, &mainnet_params()).unwrap();
    let next_value = u256_to_u128(&next);

    assert_eq!(next_value, prev_value);
}

#[test]
fn validate_difficulty_accepts_correct_target() {
    let prev = header_with_target_and_timestamp(BASE_NBITS, 1_000);

    let expected_target = calculate_next_target(&prev, 1_150, &mainnet_params()).unwrap();
    let n_bits = u256_to_compact(&expected_target);

    let current = header_with_target_and_timestamp(n_bits, 1_150);

    assert_eq!(
        validate_difficulty(Some(&prev), &current, &mainnet_params()),
        Ok(())
    );
}

#[test]
fn validate_difficulty_rejects_wrong_target() {
    let prev = header_with_target_and_timestamp(BASE_NBITS, 1_000);

    let expected_target = calculate_next_target(&prev, 1_150, &mainnet_params()).unwrap();
    let mut n_bits = u256_to_compact(&expected_target);
    n_bits = n_bits.wrapping_add(1);

    let current = header_with_target_and_timestamp(n_bits, 1_150);

    assert_eq!(
        validate_difficulty(Some(&prev), &current, &mainnet_params()),
        Err(DifficultyError::InvalidTimestamp)
    );
}

#[test]
fn genesis_block_accepts_any_target() {
    let current = header_with_target_and_timestamp(BASE_NBITS, 1_000);

    assert_eq!(
        validate_difficulty(None, &current, &mainnet_params()),
        Ok(())
    );
}

#[test]
fn multiple_adjustments_stay_within_bounds() {
    let mut headers = Vec::new();
    let first = header_with_target_and_timestamp(BASE_NBITS, 1_000);
    headers.push(first);

    for i in 1..4 {
        let prev = headers[i - 1];

        let timestamp = prev.timestamp + 300;
        let target = calculate_next_target(&prev, timestamp, &mainnet_params()).unwrap();
        let n_bits = u256_to_compact(&target);

        let header = header_with_target_and_timestamp(n_bits, timestamp);
        headers.push(header);
    }

    for i in 1..headers.len() {
        let prev_value = u256_to_u128(&headers[i - 1].target().unwrap());
        let current_value = u256_to_u128(&headers[i].target().unwrap());

        let max_increase = prev_value * 101 / 100;
        let min_decrease = prev_value * 99 / 100;

        assert!(current_value >= min_decrease);
        assert!(current_value <= max_increase);
    }
}

#[test]
fn test_min_difficulty_blocks_testnet() {
    let params = testnet_params();

    let prev_header = BlockHeader {
        version: 1,
        prev_block_hash: zero_hash(),
        merkle_root: zero_hash(),
        timestamp: 1000,
        n_bits: BASE_NBITS,
        nonce: 0,
    };

    let target_normal = calculate_next_target(&prev_header, 1150, &params).unwrap();
    assert_ne!(target_normal, params.max_target);

    let target_min = calculate_next_target(&prev_header, 1181, &params).unwrap();
    assert_eq!(target_min, params.max_target);

    let target_exactly = calculate_next_target(&prev_header, 1180, &params).unwrap();
    assert_ne!(target_exactly, params.max_target);
}

#[test]
fn test_min_difficulty_blocks_mainnet_disabled() {
    let params = mainnet_params();

    let prev_header = BlockHeader {
        version: 1,
        prev_block_hash: zero_hash(),
        merkle_root: zero_hash(),
        timestamp: 1000,
        n_bits: BASE_NBITS,
        nonce: 0,
    };

    let target = calculate_next_target(&prev_header, 2000, &params).unwrap();
    assert_ne!(target, params.max_target);
}

#[test]
fn test_min_difficulty_blocks_regtest_unchanged() {
    let params = regtest_params();

    let prev_header = BlockHeader {
        version: 1,
        prev_block_hash: zero_hash(),
        merkle_root: zero_hash(),
        timestamp: 1000,
        n_bits: BASE_NBITS,
        nonce: 0,
    };

    let target = calculate_next_target(&prev_header, 2000, &params).unwrap();
    let prev_target = prev_header.target().unwrap();
    assert_eq!(target, prev_target);
}
