use crate::primitives::hash::{sha256d, Hash256};
use crate::primitives::serialize::{Decode, DecodeError, Encode};
use crate::primitives::varint::{decode as decode_varint, encode as encode_varint, VarIntError};
use serde::{Deserialize, Serialize};

/// Maximum number of satoshis that can ever exist in the system.
pub const MAX_MONEY: u64 = 21_000_000 * 100_000_000;

/// Errors that can occur when validating basic transaction structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxError {
    NoInputs,
    NoOutputs,
    WitnessMismatch,
    ValueTooLarge,
    OutputSumOverflow,
}

/// A reference to a previous transaction output being spent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxInput {
    pub prev_txid: Hash256,
    pub prev_index: u32,
    pub script_sig: Vec<u8>,
    pub sequence: u32,
}

/// A new transaction output created by a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxOutput {
    pub value: u64,
    pub script_pubkey: Vec<u8>,
}

/// Witness data associated with a single transaction input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Witness {
    pub stack_items: Vec<Vec<u8>>,
}

/// A transaction on the Ferrous Network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub version: u32,
    pub inputs: Vec<TxInput>,
    pub outputs: Vec<TxOutput>,
    pub witnesses: Vec<Witness>,
    pub locktime: u32,
}

fn map_varint_error(err: VarIntError) -> DecodeError {
    match err {
        VarIntError::UnexpectedEof => DecodeError::UnexpectedEof,
        VarIntError::Overflow => DecodeError::Overflow,
        VarIntError::InvalidPrefix | VarIntError::NonMinimalEncoding => DecodeError::InvalidData,
    }
}

impl Encode for TxInput {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.prev_txid);
        out.extend_from_slice(&self.prev_index.encode());
        out.extend_from_slice(&self.script_sig.encode());
        out.extend_from_slice(&self.sequence.encode());
        out
    }

    fn encoded_size(&self) -> usize {
        32 + 4 + self.script_sig.encoded_size() + 4
    }
}

impl Decode for TxInput {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (prev_txid, c1) = <[u8; 32]>::decode(bytes)?;
        let (prev_index, c2) = u32::decode(&bytes[c1..])?;
        let (script_sig, c3) = Vec::<u8>::decode(&bytes[c1 + c2..])?;
        let (sequence, c4) = u32::decode(&bytes[c1 + c2 + c3..])?;

        let consumed = c1 + c2 + c3 + c4;

        Ok((
            TxInput {
                prev_txid,
                prev_index,
                script_sig,
                sequence,
            },
            consumed,
        ))
    }
}

impl Encode for TxOutput {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.value.encode());
        out.extend_from_slice(&self.script_pubkey.encode());
        out
    }

    fn encoded_size(&self) -> usize {
        8 + self.script_pubkey.encoded_size()
    }
}

impl Decode for TxOutput {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (value, c1) = u64::decode(bytes)?;
        let (script_pubkey, c2) = Vec::<u8>::decode(&bytes[c1..])?;

        Ok((
            TxOutput {
                value,
                script_pubkey,
            },
            c1 + c2,
        ))
    }
}

impl Encode for Witness {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();

        let count = self.stack_items.len() as u64;
        out.extend_from_slice(&encode_varint(count));

        for item in &self.stack_items {
            out.extend_from_slice(&item.encode());
        }

        out
    }

    fn encoded_size(&self) -> usize {
        let mut size = encode_varint(self.stack_items.len() as u64).len();
        for item in &self.stack_items {
            size += item.encoded_size();
        }
        size
    }
}

impl Decode for Witness {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (count_u64, c1) = decode_varint(bytes).map_err(map_varint_error)?;
        let count: usize = count_u64.try_into().map_err(|_| DecodeError::Overflow)?;

        let mut offset = c1;
        let mut items = Vec::with_capacity(count);

        for _ in 0..count {
            let (item, consumed) = Vec::<u8>::decode(&bytes[offset..])?;
            offset += consumed;
            items.push(item);
        }

        Ok((Witness { stack_items: items }, offset))
    }
}

impl Encode for Transaction {
    fn encode(&self) -> Vec<u8> {
        self.encode_with_witness()
    }

    fn encoded_size(&self) -> usize {
        let mut size = 4; // version
        size += crate::primitives::varint::encode(self.inputs.len() as u64).len();
        for input in &self.inputs {
            size += input.encoded_size();
        }
        size += crate::primitives::varint::encode(self.outputs.len() as u64).len();
        for output in &self.outputs {
            size += output.encoded_size();
        }
        size += 4; // locktime
        for witness in &self.witnesses {
            size += witness.encoded_size();
        }
        size
    }
}

impl Decode for Transaction {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (version, c1) = u32::decode(bytes)?;

        let (input_count_u64, c2) = decode_varint(&bytes[c1..]).map_err(map_varint_error)?;
        let input_count: usize = input_count_u64
            .try_into()
            .map_err(|_| DecodeError::Overflow)?;

        let mut offset = c1 + c2;

        let mut inputs = Vec::with_capacity(input_count);
        for _ in 0..input_count {
            let (input, consumed) = TxInput::decode(&bytes[offset..])?;
            offset += consumed;
            inputs.push(input);
        }

        let (output_count_u64, c3) = decode_varint(&bytes[offset..]).map_err(map_varint_error)?;
        let output_count: usize = output_count_u64
            .try_into()
            .map_err(|_| DecodeError::Overflow)?;

        offset += c3;

        let mut outputs = Vec::with_capacity(output_count);
        for _ in 0..output_count {
            let (output, consumed) = TxOutput::decode(&bytes[offset..])?;
            offset += consumed;
            outputs.push(output);
        }

        let (locktime, c4) = u32::decode(&bytes[offset..])?;
        offset += c4;

        if offset == bytes.len() {
            return Ok((
                Transaction {
                    version,
                    inputs,
                    outputs,
                    witnesses: Vec::new(),
                    locktime,
                },
                offset,
            ));
        }

        let mut witnesses = Vec::with_capacity(input_count);
        for _ in 0..input_count {
            let (witness, consumed) = Witness::decode(&bytes[offset..])?;
            offset += consumed;
            witnesses.push(witness);
        }

        Ok((
            Transaction {
                version,
                inputs,
                outputs,
                witnesses,
                locktime,
            },
            offset,
        ))
    }
}

impl Transaction {
    /// Computes the legacy transaction identifier excluding any witness data.
    pub fn txid(&self) -> Hash256 {
        let encoded = self.encode_without_witness();
        sha256d(&encoded)
    }

    /// Computes the witness transaction identifier including witness data.
    pub fn wtxid(&self) -> Hash256 {
        let encoded = self.encode_with_witness();
        sha256d(&encoded)
    }

    /// Encodes the transaction without any witness data.
    pub fn encode_without_witness(&self) -> Vec<u8> {
        let mut out = Vec::new();

        out.extend_from_slice(&self.version.encode());

        let input_count = self.inputs.len() as u64;
        out.extend_from_slice(&encode_varint(input_count));
        for input in &self.inputs {
            out.extend_from_slice(&input.encode());
        }

        let output_count = self.outputs.len() as u64;
        out.extend_from_slice(&encode_varint(output_count));
        for output in &self.outputs {
            out.extend_from_slice(&output.encode());
        }

        out.extend_from_slice(&self.locktime.encode());

        out
    }

    /// Encodes the transaction including witness data for each input.
    pub fn encode_with_witness(&self) -> Vec<u8> {
        let mut out = self.encode_without_witness();

        for witness in &self.witnesses {
            out.extend_from_slice(&witness.encode());
        }

        out
    }

    /// Returns true if any input carries non-empty witness data.
    pub fn has_witness(&self) -> bool {
        self.witnesses.iter().any(|w| !w.stack_items.is_empty())
    }

    /// Performs basic structural validation of the transaction.
    pub fn check_structure(&self) -> Result<(), TxError> {
        if self.inputs.is_empty() {
            return Err(TxError::NoInputs);
        }

        if self.outputs.is_empty() {
            return Err(TxError::NoOutputs);
        }

        if !self.witnesses.is_empty() && self.witnesses.len() != self.inputs.len() {
            return Err(TxError::WitnessMismatch);
        }

        let mut sum: u64 = 0;
        for output in &self.outputs {
            sum = sum
                .checked_add(output.value)
                .ok_or(TxError::OutputSumOverflow)?;
        }

        for output in &self.outputs {
            if output.value > MAX_MONEY {
                return Err(TxError::ValueTooLarge);
            }
        }

        Ok(())
    }
}
