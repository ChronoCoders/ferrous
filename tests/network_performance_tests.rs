#[cfg(test)]
mod tests {
    use ferrous_node::network::batch::{
        BroadcastCache, MessageBatcher, BATCH_INTERVAL, MAX_INV_BATCH,
    };
    use ferrous_node::network::protocol::{InvVector, NetworkAddr, INV_BLOCK};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_message_batching_inv() {
        let mut batcher = MessageBatcher::new([0xD9, 0xB4, 0xBE, 0xF9]);
        let peer_id = 1;

        // Add one item
        let item = InvVector {
            inv_type: INV_BLOCK,
            hash: [0u8; 32],
        };
        batcher.add_inv(peer_id, item);

        // Shouldn't flush immediately (neither time nor size threshold met)
        assert!(!batcher.should_flush(peer_id));

        // Force flush via public API (simulate timer)
        // Wait, should_flush checks timer.
        thread::sleep(BATCH_INTERVAL + Duration::from_millis(10));
        assert!(batcher.should_flush(peer_id));

        let msgs = batcher.flush(peer_id);
        assert_eq!(msgs.len(), 1);
        // command is [u8; 12]
        assert!(msgs[0].command.starts_with(b"inv"));
    }

    #[test]
    fn test_size_based_flush() {
        let mut batcher = MessageBatcher::new([0xD9, 0xB4, 0xBE, 0xF9]);
        let peer_id = 1;

        // Add MAX_INV_BATCH items
        for i in 0..MAX_INV_BATCH {
            let mut hash = [0u8; 32];
            hash[0] = (i % 255) as u8;
            let item = InvVector {
                inv_type: INV_BLOCK,
                hash,
            };
            batcher.add_inv(peer_id, item);
        }

        // Should flush due to size limit
        assert!(batcher.should_flush(peer_id));

        let msgs = batcher.flush(peer_id);
        assert_eq!(msgs.len(), 1); // One INV message containing vector of items
    }

    #[test]
    fn test_broadcast_cache_prevents_duplicates() {
        let mut cache = BroadcastCache::new(100);
        let peer_id = 1;
        let hash = [1u8; 32];

        assert!(!cache.already_sent(peer_id, &hash));

        cache.mark_sent(peer_id, hash);
        assert!(cache.already_sent(peer_id, &hash));
    }

    #[test]
    fn test_broadcast_cache_eviction() {
        let mut cache = BroadcastCache::new(2);
        let peer_id = 1;

        let hash1 = [1u8; 32];
        let hash2 = [2u8; 32];
        let hash3 = [3u8; 32];

        cache.mark_sent(peer_id, hash1);
        cache.mark_sent(peer_id, hash2);

        assert!(cache.already_sent(peer_id, &hash1));
        assert!(cache.already_sent(peer_id, &hash2));

        // Add 3rd item, should evict 1st
        cache.mark_sent(peer_id, hash3);

        assert!(!cache.already_sent(peer_id, &hash1)); // Evicted
        assert!(cache.already_sent(peer_id, &hash2));
        assert!(cache.already_sent(peer_id, &hash3));
    }

    #[test]
    fn test_batch_clear_on_disconnect() {
        let mut batcher = MessageBatcher::new([0xD9, 0xB4, 0xBE, 0xF9]);
        let peer_id = 1;

        let item = InvVector {
            inv_type: INV_BLOCK,
            hash: [0u8; 32],
        };
        batcher.add_inv(peer_id, item);

        batcher.clear_peer(peer_id);

        // Even after time, should be empty
        thread::sleep(BATCH_INTERVAL + Duration::from_millis(10));
        assert!(!batcher.should_flush(peer_id));

        let msgs = batcher.flush(peer_id);
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_multi_peer_batching() {
        let mut batcher = MessageBatcher::new([0xD9, 0xB4, 0xBE, 0xF9]);
        let peer1 = 1;
        let peer2 = 2;

        let item = InvVector {
            inv_type: INV_BLOCK,
            hash: [0u8; 32],
        };

        batcher.add_inv(peer1, item);
        batcher.add_inv(peer2, item);

        thread::sleep(BATCH_INTERVAL + Duration::from_millis(10));

        let batches = batcher.flush_all_needed();
        assert_eq!(batches.len(), 2);
        assert!(batches.contains_key(&peer1));
        assert!(batches.contains_key(&peer2));
    }

    #[test]
    fn test_empty_batch_doesnt_send() {
        let mut batcher = MessageBatcher::new([0xD9, 0xB4, 0xBE, 0xF9]);
        let peer_id = 1;

        // Don't add anything
        thread::sleep(BATCH_INTERVAL + Duration::from_millis(10));

        assert!(!batcher.should_flush(peer_id));
        let msgs = batcher.flush(peer_id);
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_mixed_batch_types() {
        let mut batcher = MessageBatcher::new([0xD9, 0xB4, 0xBE, 0xF9]);
        let peer_id = 1;

        let inv_item = InvVector {
            inv_type: INV_BLOCK,
            hash: [0u8; 32],
        };
        batcher.add_inv(peer_id, inv_item);
        batcher.add_getdata(peer_id, inv_item);

        thread::sleep(BATCH_INTERVAL + Duration::from_millis(10));

        let msgs = batcher.flush(peer_id);
        assert_eq!(msgs.len(), 2); // One INV, one GETDATA

        let commands: Vec<&[u8]> = msgs.iter().map(|m| &m.command[..]).collect();
        assert!(commands.iter().any(|c| c.starts_with(b"inv")));
        assert!(commands.iter().any(|c| c.starts_with(b"getdata")));
    }

    #[test]
    fn test_addr_batching() {
        let mut batcher = MessageBatcher::new([0xD9, 0xB4, 0xBE, 0xF9]);
        let peer_id = 1;

        let addr = NetworkAddr {
            timestamp: 0,
            services: 1,
            ip: [0u8; 16],
            port: 8333,
        };

        batcher.add_addr(peer_id, addr);

        thread::sleep(BATCH_INTERVAL + Duration::from_millis(10));
        let msgs = batcher.flush(peer_id);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].command.starts_with(b"addr"));
    }

    #[test]
    fn test_cache_clear_on_disconnect() {
        let mut cache = BroadcastCache::new(100);
        let peer_id = 1;
        let hash = [1u8; 32];

        cache.mark_sent(peer_id, hash);
        assert!(cache.already_sent(peer_id, &hash));

        cache.clear_peer(peer_id);
        assert!(!cache.already_sent(peer_id, &hash));
    }
}
