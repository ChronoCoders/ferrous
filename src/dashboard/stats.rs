use std::time::{Duration, Instant};

use crate::mining::MiningEvent;

#[derive(Clone)]
pub struct BlockInfo {
    pub height: u32,
    pub hash: String,
    pub nonce: u64,
    pub core: u64,
    pub timestamp: Instant,
}

pub struct MiningStats {
    pub start_time: Instant,
    pub blocks_mined: u64,
    pub current_height: u32,
    pub current_hash: String,
    pub difficulty: String,
    pub network: String,
    pub recent_blocks: Vec<BlockInfo>,
    pub last_block_time: Option<Instant>,
    pub hash_rate: f64,
}

impl BlockInfo {
    pub fn from_event(event: MiningEvent) -> Self {
        Self {
            height: event.height,
            hash: event.hash,
            nonce: event.nonce,
            core: event.worker_id,
            timestamp: Instant::now(),
        }
    }
}

impl MiningStats {
    pub fn new(network: String) -> Self {
        Self {
            start_time: Instant::now(),
            blocks_mined: 0,
            current_height: 0,
            current_hash: String::new(),
            difficulty: String::new(),
            network,
            recent_blocks: Vec::new(),
            last_block_time: None,
            hash_rate: 0.0,
        }
    }

    pub fn add_block(&mut self, block: BlockInfo) {
        self.blocks_mined += 1;
        self.last_block_time = Some(block.timestamp);
        self.current_height = block.height;
        self.current_hash = block.hash.clone();
        self.recent_blocks.insert(0, block);
        if self.recent_blocks.len() > 5 {
            self.recent_blocks.truncate(5);
        }
    }

    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }
}
