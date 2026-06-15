use crate::consensus::transaction::{Transaction, TxOutput};
use crate::primitives::hash::Hash256;
use crate::primitives::serialize::{Decode, DecodeError, Encode};
use crate::script::engine::{validate_p2dl, ScriptContext};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OutPoint {
    pub txid: Hash256,
    pub vout: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtxoEntry {
    pub output: TxOutput,
    pub coinbase: bool,
    pub height: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtxoEntryV2 {
    pub commitment: [u8; 32],
    pub script_pubkey: Vec<u8>,
    pub encrypted_amount: Vec<u8>,
    pub ephemeral_pubkey: [u8; 32],
    pub coinbase: bool,
    pub height: u32,
}

impl Encode for UtxoEntryV2 {
    fn encode(&self) -> Vec<u8> {
        let mut out = self.commitment.to_vec();
        out.extend_from_slice(&self.script_pubkey.encode());
        out.extend_from_slice(&self.encrypted_amount.encode());
        out.extend_from_slice(&self.ephemeral_pubkey);
        out.push(self.coinbase as u8);
        out.extend_from_slice(&self.height.encode());
        out
    }

    fn encoded_size(&self) -> usize {
        32 + self.script_pubkey.encoded_size() + self.encrypted_amount.encoded_size() + 32 + 1 + 4
    }
}

impl Decode for UtxoEntryV2 {
    fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let (commitment, c1) = <[u8; 32]>::decode(bytes)?;
        let (script_pubkey, c2) = Vec::<u8>::decode(&bytes[c1..])?;
        let (encrypted_amount, c3) = Vec::<u8>::decode(&bytes[c1 + c2..])?;
        let (ephemeral_pubkey, c4) = <[u8; 32]>::decode(&bytes[c1 + c2 + c3..])?;
        let (cb_byte, c5) = u8::decode(&bytes[c1 + c2 + c3 + c4..])?;
        let (height, c6) = u32::decode(&bytes[c1 + c2 + c3 + c4 + c5..])?;
        Ok((
            UtxoEntryV2 {
                commitment,
                script_pubkey,
                encrypted_amount,
                ephemeral_pubkey,
                coinbase: cb_byte != 0,
                height,
            },
            c1 + c2 + c3 + c4 + c5 + c6,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct UtxoSet {
    pub(crate) utxos: HashMap<OutPoint, UtxoEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UtxoError {
    UtxoNotFound,
    UtxoAlreadySpent,
    DuplicateUtxo,
    CoinbaseNotMature,
    ImmatureCoinbase,
    ScriptValidationFailed,
    InsufficientValue,
    ValueOverflow,
}

impl UtxoSet {
    pub fn new() -> Self {
        Self {
            utxos: HashMap::new(),
        }
    }

    pub fn insert(&mut self, outpoint: OutPoint, entry: UtxoEntry) {
        self.utxos.insert(outpoint, entry);
    }

    pub fn remove(&mut self, outpoint: &OutPoint) {
        self.utxos.remove(outpoint);
    }

    pub fn add_transaction(
        &mut self,
        tx: &Transaction,
        height: u32,
        is_coinbase: bool,
    ) -> Result<(), UtxoError> {
        let txid = tx.txid();

        for (index, output) in tx.outputs.iter().enumerate() {
            let outpoint = OutPoint {
                txid,
                vout: index as u32,
            };

            if self.utxos.contains_key(&outpoint) {
                return Err(UtxoError::DuplicateUtxo);
            }

            self.utxos.insert(
                outpoint,
                UtxoEntry {
                    output: output.clone(),
                    height: height as u64,
                    coinbase: is_coinbase,
                },
            );
        }

        Ok(())
    }

    pub fn spend_input(
        &mut self,
        outpoint: &OutPoint,
        current_height: u32,
    ) -> Result<UtxoEntry, UtxoError> {
        let entry = self.utxos.remove(outpoint).ok_or(UtxoError::UtxoNotFound)?;

        if entry.coinbase {
            let confirmations = (current_height as u64).saturating_sub(entry.height);

            if confirmations < crate::consensus::validation::COINBASE_MATURITY {
                self.utxos.insert(*outpoint, entry);
                return Err(UtxoError::CoinbaseNotMature);
            }
        }

        Ok(entry)
    }

    pub fn get(&self, outpoint: &OutPoint) -> Option<&UtxoEntry> {
        self.utxos.get(outpoint)
    }

    pub fn contains(&self, outpoint: &OutPoint) -> bool {
        self.utxos.contains_key(outpoint)
    }

    pub fn apply_transaction(
        &mut self,
        tx: &Transaction,
        height: u32,
        is_coinbase: bool,
    ) -> Result<Vec<UtxoEntry>, UtxoError> {
        let mut spent = Vec::new();

        if !is_coinbase {
            for input in &tx.inputs {
                let outpoint = OutPoint {
                    txid: input.prev_txid,
                    vout: input.prev_index,
                };

                let entry = self.spend_input(&outpoint, height)?;
                spent.push(entry);
            }
        }

        if !is_coinbase && !spent.is_empty() {
            let spent_outputs: Vec<TxOutput> = spent.iter().map(|e| e.output.clone()).collect();

            for (index, input) in tx.inputs.iter().enumerate() {
                let script_pubkey = &spent_outputs[index].script_pubkey;

                let context = ScriptContext {
                    transaction: tx,
                    input_index: index,
                    spent_outputs: &spent_outputs,
                };

                let is_p2dl = script_pubkey.len() == 36
                    && script_pubkey[0] == 0xaa
                    && script_pubkey[1] == 0x20
                    && script_pubkey[34] == 0x88
                    && script_pubkey[35] == 0xac;

                if is_p2dl {
                    let result = validate_p2dl(&input.script_sig, script_pubkey, &context)
                        .map_err(|_| UtxoError::ScriptValidationFailed)?;
                    if !result {
                        return Err(UtxoError::ScriptValidationFailed);
                    }
                }
            }
        }

        self.add_transaction(tx, height, is_coinbase)?;

        Ok(spent)
    }
}

impl Default for UtxoSet {
    fn default() -> Self {
        Self::new()
    }
}
