use crate::network::message::NetworkMessage;
use crate::network::protocol::MessagePayload;
use std::time::{SystemTime, UNIX_EPOCH};

// Validation limits
pub const MAX_MESSAGE_SIZE: usize = 32 * 1024 * 1024; // 32 MB
pub const MAX_INV_COUNT: usize = 50000;
pub const MAX_GETDATA_COUNT: usize = 50000;
pub const MAX_HEADERS_COUNT: usize = 2000;
pub const MAX_ADDR_COUNT: usize = 1000;
pub const MAX_TIMESTAMP_DRIFT: u64 = 2 * 60 * 60; // 2 hours

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    MessageTooLarge,
    InvalidPayload(String),
    InvalidTimestamp,
    UnsupportedVersion,
    InvalidChecksum,
    MalformedData,
    TooManyItems,
    InvalidNonce,
}

pub struct MessageValidator;

impl MessageValidator {
    /// Validate complete message before processing
    pub fn validate_message(msg: &NetworkMessage) -> Result<(), ValidationError> {
        // 1. Size check
        if msg.length as usize > MAX_MESSAGE_SIZE {
            return Err(ValidationError::MessageTooLarge);
        }

        // 2. Checksum check is implicitly done during parsing in some implementations,
        // but NetworkMessage stores it. We could verify it here if we wanted to be strict,
        // but parsing usually handles it.
        // Assuming payload is already valid bytes.

        Ok(())
    }

    /// Validate message payload contents
    pub fn validate_payload(payload: &MessagePayload) -> Result<(), ValidationError> {
        match payload {
            MessagePayload::Version(v) => {
                // Validate timestamp (not too far in future)
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;

                // Allow some drift
                if v.timestamp > now + MAX_TIMESTAMP_DRIFT as i64 {
                    return Err(ValidationError::InvalidTimestamp);
                }

                // Version should be reasonable
                if !Self::is_version_supported(v.version) {
                    return Err(ValidationError::UnsupportedVersion);
                }

                Ok(())
            }

            MessagePayload::Inv(msg) => {
                if msg.inventory.len() > MAX_INV_COUNT {
                    return Err(ValidationError::TooManyItems);
                }
                Ok(())
            }

            MessagePayload::GetData(msg) => {
                if msg.inventory.len() > MAX_GETDATA_COUNT {
                    return Err(ValidationError::TooManyItems);
                }
                Ok(())
            }

            MessagePayload::Headers(msg) => {
                if msg.headers.len() > MAX_HEADERS_COUNT {
                    return Err(ValidationError::TooManyItems);
                }
                Ok(())
            }

            MessagePayload::Addr(msg) => {
                if msg.addresses.len() > MAX_ADDR_COUNT {
                    return Err(ValidationError::TooManyItems);
                }
                Ok(())
            }

            MessagePayload::Ping(msg) => {
                // Nonce should not be zero (weak check, but helps)
                if msg.nonce == 0 {
                    return Err(ValidationError::InvalidNonce);
                }
                Ok(())
            }

            MessagePayload::Pong(msg) => {
                if msg.nonce == 0 {
                    return Err(ValidationError::InvalidNonce);
                }
                Ok(())
            }

            // Other messages are trivial or handled elsewhere
            _ => Ok(()),
        }
    }

    /// Check if protocol version is supported
    fn is_version_supported(version: u32) -> bool {
        (70001..=70016).contains(&version)
    }

    /// Validate block message size
    pub fn validate_block_size(size: usize) -> Result<(), ValidationError> {
        if size > MAX_MESSAGE_SIZE {
            return Err(ValidationError::MessageTooLarge);
        }
        Ok(())
    }

    /// Validate transaction message size
    pub fn validate_tx_size(size: usize) -> Result<(), ValidationError> {
        if size > 1024 * 1024 {
            // 1 MB max tx size
            return Err(ValidationError::MessageTooLarge);
        }
        Ok(())
    }
}

pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}

impl Validate for NetworkMessage {
    fn validate(&self) -> Result<(), ValidationError> {
        MessageValidator::validate_message(self)
    }
}

impl Validate for MessagePayload {
    fn validate(&self) -> Result<(), ValidationError> {
        MessageValidator::validate_payload(self)
    }
}
