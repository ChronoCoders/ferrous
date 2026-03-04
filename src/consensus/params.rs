use crate::consensus::block::U256;

#[derive(Debug, Clone)]
pub struct ChainParams {
    pub target_block_time: u64,
    pub max_target: U256,
    pub difficulty_adjustment: bool,
    pub allow_min_difficulty_blocks: bool,
}

pub enum Network {
    Mainnet,
    Testnet,
    Regtest,
}

impl Network {
    pub fn params(&self) -> ChainParams {
        match self {
            Network::Mainnet => ChainParams {
                target_block_time: 150,
                max_target: crate::consensus::difficulty::MAINNET_MAX_TARGET,
                difficulty_adjustment: true,
                allow_min_difficulty_blocks: false,
            },
            Network::Testnet => ChainParams {
                target_block_time: 150,
                max_target: crate::consensus::difficulty::TESTNET_MAX_TARGET,
                difficulty_adjustment: true,
                allow_min_difficulty_blocks: true,
            },
            Network::Regtest => ChainParams {
                target_block_time: 1,
                max_target: crate::consensus::difficulty::REGTEST_MAX_TARGET,
                difficulty_adjustment: false,
                allow_min_difficulty_blocks: true,
            },
        }
    }
}
