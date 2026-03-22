use crate::consensus::block::U256;

#[derive(Debug, Clone)]
pub struct ChainParams {
    pub target_block_time: u64,
    pub max_target: U256,
    pub difficulty_adjustment: bool,
    pub allow_min_difficulty_blocks: bool,
    /// n_bits for the genesis block. Sets the starting difficulty independently
    /// of max_target (the difficulty floor). For testnet this is calibrated to
    /// ~150s block time at the expected hashrate so the chain doesn't oscillate
    /// through hundreds of ramp-up blocks on every fresh start.
    pub genesis_n_bits: u32,
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
                // Bitcoin mainnet difficulty-1 (0x1d00ffff) — very hard, as intended.
                genesis_n_bits: 0x1d00_ffff,
            },
            Network::Testnet => ChainParams {
                target_block_time: 150,
                max_target: crate::consensus::difficulty::TESTNET_MAX_TARGET,
                difficulty_adjustment: true,
                allow_min_difficulty_blocks: false,
                // Calibrated for ~150s blocks at ~1.3 MH/s combined hashrate.
                // n_bits = 0x1D161C29 → target = 0x161C29 * 2^208
                // Expected block time = (2^48 / 0x161C29) / 1_300_000 ≈ 149s
                // Prevents hundreds of ramp-up blocks at near-zero difficulty on
                // every fresh testnet start.
                genesis_n_bits: 0x1D16_1C29,
            },
            Network::Regtest => ChainParams {
                target_block_time: 1,
                max_target: crate::consensus::difficulty::REGTEST_MAX_TARGET,
                difficulty_adjustment: false,
                allow_min_difficulty_blocks: true,
                genesis_n_bits: 0x207f_ffff, // trivial — instant mining for tests
            },
        }
    }
}
