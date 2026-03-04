use crate::consensus::transaction::{Transaction, TxOutput};
use crate::primitives::hash::Hash256;
use crate::script::engine::{validate_p2pkh, validate_p2wpkh, ScriptContext};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OutPoint {
    pub txid: Hash256,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtxoEntry {
    pub output: TxOutput,
    pub height: u32,
    pub is_coinbase: bool,
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
    ScriptValidationFailed,
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
                index: index as u32,
            };

            if self.utxos.contains_key(&outpoint) {
                return Err(UtxoError::DuplicateUtxo);
            }

            self.utxos.insert(
                outpoint,
                UtxoEntry {
                    output: output.clone(),
                    height,
                    is_coinbase,
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

        if entry.is_coinbase {
            let confirmations = current_height.saturating_sub(entry.height);

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
                    index: input.prev_index,
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

                let is_p2pkh = script_pubkey.len() == 25
                    && script_pubkey[0] == 0x76
                    && script_pubkey[1] == 0xa9
                    && script_pubkey[2] == 0x14
                    && script_pubkey[23] == 0x88
                    && script_pubkey[24] == 0xac;

                let is_p2wpkh = script_pubkey.len() == 22
                    && script_pubkey[0] == 0x00
                    && script_pubkey[1] == 0x14;

                if is_p2pkh {
                    let result = validate_p2pkh(&input.script_sig, script_pubkey, &context)
                        .map_err(|_| UtxoError::ScriptValidationFailed)?;
                    if !result {
                        return Err(UtxoError::ScriptValidationFailed);
                    }
                } else if is_p2wpkh {
                    let witness = tx
                        .witnesses
                        .get(index)
                        .ok_or(UtxoError::ScriptValidationFailed)?;
                    let result = validate_p2wpkh(witness, script_pubkey, &context)
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
