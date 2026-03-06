#[cfg(test)]
mod tests {
    use ferrous_node::network::message::{NetworkMessage, REGTEST_MAGIC};
    use ferrous_node::network::protocol::{
        AddrMessage, GetDataMessage, HeadersMessage, InvMessage, InvVector, MessagePayload,
        NetAddr, NetworkAddr, PingMessage, VersionMessage, INV_BLOCK,
    };
    use ferrous_node::network::validation::{
        Validate, ValidationError, MAX_ADDR_COUNT, MAX_GETDATA_COUNT, MAX_HEADERS_COUNT,
        MAX_INV_COUNT, MAX_MESSAGE_SIZE, MAX_TIMESTAMP_DRIFT,
    };
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_dummy_net_addr() -> NetAddr {
        NetAddr::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8333),
            1,
        )
    }

    #[test]
    fn test_valid_message_size() {
        let payload = vec![0u8; 100];
        let msg = NetworkMessage::new(REGTEST_MAGIC, "test", payload);
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn test_invalid_message_size() {
        // Create a message logically larger than MAX_MESSAGE_SIZE
        // We can't easily allocate 32MB+ in test, but we can construct NetworkMessage with fake length
        // provided we don't encode it.
        let mut msg = NetworkMessage::new(REGTEST_MAGIC, "test", vec![]);
        msg.length = (MAX_MESSAGE_SIZE + 1) as u32;
        assert_eq!(msg.validate(), Err(ValidationError::MessageTooLarge));
    }

    #[test]
    fn test_valid_version_payload() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let msg = VersionMessage {
            version: 70015,
            services: 1,
            timestamp: now,
            receiver: create_dummy_net_addr(),
            sender: create_dummy_net_addr(),
            nonce: 12345,
            user_agent: "/Test:0.1/".to_string(),
            start_height: 0,
        };

        let payload = MessagePayload::Version(msg);
        assert!(payload.validate().is_ok());
    }

    #[test]
    fn test_invalid_version_payload_timestamp() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let msg = VersionMessage {
            version: 70015,
            services: 1,
            timestamp: now + (MAX_TIMESTAMP_DRIFT as i64) + 10, // Too far in future
            receiver: create_dummy_net_addr(),
            sender: create_dummy_net_addr(),
            nonce: 12345,
            user_agent: "/Test:0.1/".to_string(),
            start_height: 0,
        };

        let payload = MessagePayload::Version(msg);
        assert_eq!(payload.validate(), Err(ValidationError::InvalidTimestamp));
    }

    #[test]
    fn test_invalid_version_payload_version() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let msg = VersionMessage {
            version: 1, // Too low
            services: 1,
            timestamp: now,
            receiver: create_dummy_net_addr(),
            sender: create_dummy_net_addr(),
            nonce: 12345,
            user_agent: "/Test:0.1/".to_string(),
            start_height: 0,
        };

        let payload = MessagePayload::Version(msg);
        assert_eq!(payload.validate(), Err(ValidationError::UnsupportedVersion));
    }

    #[test]
    fn test_inv_count_limit() {
        let inventory = vec![
            InvVector {
                inv_type: INV_BLOCK,
                hash: [0u8; 32],
            };
            MAX_INV_COUNT + 1
        ];

        let msg = InvMessage { inventory };
        let payload = MessagePayload::Inv(msg);
        assert_eq!(payload.validate(), Err(ValidationError::TooManyItems));
    }

    #[test]
    fn test_getdata_count_limit() {
        let inventory = vec![
            InvVector {
                inv_type: INV_BLOCK,
                hash: [0u8; 32],
            };
            MAX_GETDATA_COUNT + 1
        ];

        let msg = GetDataMessage { inventory };
        let payload = MessagePayload::GetData(msg);
        assert_eq!(payload.validate(), Err(ValidationError::TooManyItems));
    }

    #[test]
    fn test_headers_count_limit() {
        // We need dummy BlockHeader
        let header = ferrous_node::consensus::block::BlockHeader {
            version: 1,
            prev_block_hash: [0u8; 32],
            merkle_root: [0u8; 32],
            timestamp: 0,
            n_bits: 0,
            nonce: 0,
        };

        let headers = vec![header; MAX_HEADERS_COUNT + 1];
        let msg = HeadersMessage { headers };
        let payload = MessagePayload::Headers(msg);
        assert_eq!(payload.validate(), Err(ValidationError::TooManyItems));
    }

    #[test]
    fn test_addr_count_limit() {
        let addr = NetworkAddr {
            timestamp: 0,
            services: 1,
            ip: [0u8; 16],
            port: 8333,
        };

        let addresses = vec![addr; MAX_ADDR_COUNT + 1];
        let msg = AddrMessage { addresses };
        let payload = MessagePayload::Addr(msg);
        assert_eq!(payload.validate(), Err(ValidationError::TooManyItems));
    }

    #[test]
    fn test_invalid_ping_nonce() {
        let msg = PingMessage { nonce: 0 };
        let payload = MessagePayload::Ping(msg);
        assert_eq!(payload.validate(), Err(ValidationError::InvalidNonce));
    }

    #[test]
    fn test_valid_ping_nonce() {
        let msg = PingMessage { nonce: 12345 };
        let payload = MessagePayload::Ping(msg);
        assert!(payload.validate().is_ok());
    }
}
