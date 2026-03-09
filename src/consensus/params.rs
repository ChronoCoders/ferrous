use crate::consensus::block::U256;

#[derive(Debug, Clone)]
pub struct ChainParams {
    pub target_block_time: u64,
    pub max_target: U256,
    pub difficulty_adjustment: bool,
    pub allow_min_difficulty_blocks: bool,
}

#[derive(Debug, Clone)]
pub enum Network {
    Mainnet,
    Testnet,
    Regtest,
}

impl Network {
    pub fn prefix(&self) -> u8 {
        match self {
            Network::Mainnet => 0x00,
            Network::Testnet => 0x6f,
            Network::Regtest => 0x6f,
        }
    }

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
