use crate::consensus::chain::ChainState;
use crate::consensus::transaction::{Transaction, TxInput, TxOutput};
use crate::script::sighash::compute_sighash;
use crate::wallet::address::address_to_script_pubkey;
use crate::wallet::manager::{Wallet, WalletUtxo};
use secp256k1::{Message, Secp256k1};

pub struct TransactionBuilder;

impl TransactionBuilder {
    pub fn create_transaction(
        wallet: &mut Wallet,
        chain: &ChainState,
        to_address: &str,
        amount: u64,
        fee: u64,
    ) -> Result<Transaction, String> {
        if amount == 0 {
            return Err("Amount must be positive".to_string());
        }

        let total_needed = amount
            .checked_add(fee)
            .ok_or_else(|| "Amount overflow".to_string())?;

        // get_utxos() returns UTXOs sorted descending by value; select_coins
        // iterates in that order without re-sorting.
        let utxos = wallet.get_utxos(chain)?;

        let (selected, change) = select_coins(utxos, total_needed)?;

        let to_script = address_to_script_pubkey(to_address)?;

        let mut outputs = Vec::new();
        outputs.push(TxOutput {
            value: amount,
            script_pubkey: to_script,
        });

        if change > 0 {
            let change_address = wallet.generate_change_address()?;
            let change_script = address_to_script_pubkey(&change_address)?;
            outputs.push(TxOutput {
                value: change,
                script_pubkey: change_script,
            });
        }

        let mut inputs = Vec::new();
        let mut spent_outputs = Vec::new();

        for u in &selected {
            inputs.push(TxInput {
                prev_txid: u.txid,
                prev_index: u.vout,
                script_sig: Vec::new(),
                sequence: 0xFFFF_FFFF,
            });
            spent_outputs.push(TxOutput {
                value: u.value,
                script_pubkey: u.script_pubkey.clone(),
            });
        }

        let mut tx = Transaction {
            version: 1,
            inputs,
            outputs,
            witnesses: Vec::new(),
            locktime: 0,
        };

        sign_transaction(&mut tx, &spent_outputs, wallet)?;

        Ok(tx)
    }
}

fn select_coins(
    utxos: Vec<WalletUtxo>,
    total_needed: u64,
) -> Result<(Vec<WalletUtxo>, u64), String> {
    let mut selected = Vec::new();
    let mut total = 0u64;

    for u in utxos {
        selected.push(u);
        total = total
            .checked_add(selected.last().unwrap().value)
            .ok_or_else(|| "Coin selection overflow".to_string())?;
        if total >= total_needed {
            break;
        }
    }

    if total < total_needed {
        return Err("Insufficient funds".to_string());
    }

    let change = total - total_needed;
    Ok((selected, change))
}

fn sign_transaction(
    tx: &mut Transaction,
    spent_outputs: &[TxOutput],
    wallet: &Wallet,
) -> Result<(), String> {
    if tx.inputs.is_empty() {
        return Ok(());
    }

    let secp = Secp256k1::new();

    let mut script_sigs: Vec<Vec<u8>> = Vec::with_capacity(tx.inputs.len());

    for (index, _) in tx.inputs.iter().enumerate() {
        let spent_output = &spent_outputs[index];

        let address = wallet
            .find_address_for_script(&spent_output.script_pubkey)
            .ok_or_else(|| "Address not in wallet".to_string())?;

        let private_key = wallet
            .get_private_key(&address)
            .ok_or_else(|| "Private key not found".to_string())?;

        let sighash = compute_sighash(tx, index, spent_outputs)
            .map_err(|e| format!("Sighash error: {:?}", e))?;
        let message =
            Message::from_digest_slice(&sighash).map_err(|e| format!("Message error: {}", e))?;
        let sig = secp.sign_ecdsa(&message, private_key.inner());
        let der_sig = sig.serialize_der();
        let mut sig_with_hashtype = der_sig.to_vec();
        sig_with_hashtype.push(0x01);
        let mut script_sig = Vec::new();
        script_sig.push(sig_with_hashtype.len() as u8);
        script_sig.extend_from_slice(&sig_with_hashtype);
        let pubkey = private_key.public_key_bytes();
        script_sig.push(pubkey.len() as u8);
        script_sig.extend_from_slice(&pubkey);
        script_sigs.push(script_sig);
    }

    for (index, script_sig) in script_sigs.into_iter().enumerate() {
        tx.inputs[index].script_sig = script_sig;
    }

    Ok(())
}
