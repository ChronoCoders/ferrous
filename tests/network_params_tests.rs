use ferrous_node::consensus::difficulty::{
    MAINNET_MAX_TARGET, MAINNET_TARGET_BLOCK_TIME, REGTEST_MAX_TARGET, TESTNET_MAX_TARGET,
};
use ferrous_node::consensus::params::Network;

#[test]
fn mainnet_params_match_defaults() {
    let params = Network::Mainnet.params();

    assert_eq!(params.target_block_time, MAINNET_TARGET_BLOCK_TIME);
    assert_eq!(params.max_target, MAINNET_MAX_TARGET);
    assert!(params.difficulty_adjustment);
    assert!(!params.allow_min_difficulty_blocks);
}

#[test]
fn testnet_params_allow_min_difficulty() {
    let params = Network::Testnet.params();

    assert_eq!(params.target_block_time, MAINNET_TARGET_BLOCK_TIME);
    assert_eq!(params.max_target, TESTNET_MAX_TARGET);
    assert!(params.difficulty_adjustment);
    assert!(params.allow_min_difficulty_blocks);
}

#[test]
fn regtest_params_disable_adjustment() {
    let params = Network::Regtest.params();

    assert_eq!(params.target_block_time, 1);
    assert_eq!(params.max_target, REGTEST_MAX_TARGET);
    assert!(!params.difficulty_adjustment);
    assert!(params.allow_min_difficulty_blocks);
}
