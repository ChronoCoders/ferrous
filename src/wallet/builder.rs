use crate::consensus::chain::ChainState;
use crate::consensus::transaction::{
    BlindingFactor, Transaction, TransactionV2, TxInput, TxInputV2, TxOutput, TxOutputV2,
    TX_VERSION_V2,
};
use crate::consensus::utxo::OutPoint;
use crate::crypto::commitments::{commit, encrypt_amount, generate_range_proof};
use crate::primitives::hash::Hash256;
use crate::script::sighash::{compute_sighash, compute_sighash_v2};
use crate::wallet::address::address_to_script_pubkey;
use crate::wallet::dilithium::DilithiumKeypair;
use crate::wallet::keys::{derive_blinding, derive_view_key};
use crate::wallet::manager::{Wallet, WalletUtxo};
use curve25519_dalek_ng::ristretto::RistrettoPoint;
use curve25519_dalek_ng::scalar::Scalar;
use rand::rngs::OsRng;
use rand::RngCore;
use std::collections::HashSet;

pub struct TransactionBuilder;

impl TransactionBuilder {
    pub fn create_transaction(
        wallet: &mut Wallet,
        chain: &ChainState,
        to_address: &str,
        amount: u64,
        fee: u64,
        spent_in_mempool: &HashSet<(Hash256, u32)>,
    ) -> Result<Transaction, String> {
        if amount == 0 {
            return Err("Amount must be positive".to_string());
        }

        let total_needed = amount
            .checked_add(fee)
            .ok_or_else(|| "Amount overflow".to_string())?;

        // get_utxos() returns UTXOs sorted descending by value; select_coins
        // iterates in that order without re-sorting.
        let utxos = wallet.get_utxos(chain, spent_in_mempool)?;

        let (selected, change) = select_coins(utxos, total_needed)?;

        let to_script = address_to_script_pubkey(to_address)?;

        let mut outputs = Vec::new();
        outputs.push(TxOutput {
            value: amount,
            script_pubkey: to_script,
        });

        if change > 0 {
            let change_address = wallet.get_or_create_change_address()?;
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

    pub fn build_v2_transaction(
        wallet: &mut Wallet,
        chain: &ChainState,
        to_address: &str,
        recipient_view_pubkey: &RistrettoPoint,
        amount: u64,
        fee: u64,
        spent_in_mempool: &HashSet<(Hash256, u32)>,
    ) -> Result<TransactionV2, String> {
        if amount == 0 {
            return Err("Amount must be positive".to_string());
        }

        let seed = wallet
            .bip39_seed()
            .ok_or_else(|| "Confidential transactions require a seeded wallet".to_string())?;

        let total_needed = amount
            .checked_add(fee)
            .ok_or_else(|| "Amount overflow".to_string())?;

        let utxos = wallet.get_utxos(chain, spent_in_mempool)?;
        let (selected, change) = select_coins(utxos, total_needed)?;

        let mut inputs = Vec::with_capacity(selected.len());
        let mut spent_scripts = Vec::with_capacity(selected.len());
        for u in &selected {
            inputs.push(TxInputV2 {
                prev_txid: u.txid,
                prev_index: u.vout,
                script_sig: Vec::new(),
                sequence: 0xFFFF_FFFF,
            });
            spent_scripts.push(u.script_pubkey.clone());
        }

        let pay_op = OutPoint {
            txid: selected[0].txid,
            vout: selected[0].vout,
        };
        let pay_blind = if change > 0 {
            derive_blinding(&seed, &pay_op)
        } else {
            BlindingFactor((-random_balancing_scalar()).to_bytes())
        };

        let to_script = address_to_script_pubkey(to_address)?;
        let pay_commitment = commit(amount, &pay_blind);
        let pay_proof = generate_range_proof(amount, &pay_blind)
            .map_err(|e| format!("Range proof (payment): {:?}", e))?;
        let (pay_enc, pay_eph) = encrypt_amount(amount, &pay_blind, recipient_view_pubkey);

        let change_blind = BlindingFactor((-Scalar::from_bytes_mod_order(pay_blind.0)).to_bytes());
        let change_address = wallet.get_or_create_change_address()?;
        let change_script = address_to_script_pubkey(&change_address)?;
        let change_commitment = commit(change, &change_blind);
        let change_proof = generate_range_proof(change, &change_blind)
            .map_err(|e| format!("Range proof (change): {:?}", e))?;
        let (_, own_view_pubkey) = derive_view_key(&seed);
        let (change_enc, change_eph) = encrypt_amount(change, &change_blind, &own_view_pubkey);

        let outputs = vec![
            TxOutputV2 {
                commitment: pay_commitment,
                range_proof: pay_proof,
                script_pubkey: to_script,
                encrypted_amount: pay_enc,
                ephemeral_pubkey: pay_eph,
            },
            TxOutputV2 {
                commitment: change_commitment,
                range_proof: change_proof,
                script_pubkey: change_script,
                encrypted_amount: change_enc,
                ephemeral_pubkey: change_eph,
            },
        ];

        let mut tx = TransactionV2 {
            version: TX_VERSION_V2,
            inputs,
            outputs,
            fee,
            locktime: 0,
        };

        sign_v2_transaction(&mut tx, &spent_scripts, wallet)?;

        Ok(tx)
    }

    pub fn build_funding_tx(
        wallet: &mut Wallet,
        to_address: &str,
        recipient_view_pubkey: &RistrettoPoint,
        v1_utxo: &WalletUtxo,
        fee: u64,
    ) -> Result<TransactionV2, String> {
        let value = v1_utxo
            .value
            .checked_sub(fee)
            .ok_or_else(|| "Fee exceeds input value".to_string())?;
        if value == 0 {
            return Err("Funding output must be positive".to_string());
        }

        let blind = BlindingFactor([0u8; 32]);
        let to_script = address_to_script_pubkey(to_address)?;
        let commitment = commit(value, &blind);
        let proof = generate_range_proof(value, &blind)
            .map_err(|e| format!("Range proof (funding): {:?}", e))?;
        let (enc, eph) = encrypt_amount(value, &blind, recipient_view_pubkey);

        let mut tx = TransactionV2 {
            version: TX_VERSION_V2,
            inputs: vec![TxInputV2 {
                prev_txid: v1_utxo.txid,
                prev_index: v1_utxo.vout,
                script_sig: Vec::new(),
                sequence: 0xFFFF_FFFF,
            }],
            outputs: vec![TxOutputV2 {
                commitment,
                range_proof: proof,
                script_pubkey: to_script,
                encrypted_amount: enc,
                ephemeral_pubkey: eph,
            }],
            fee,
            locktime: 0,
        };

        sign_v2_transaction(
            &mut tx,
            std::slice::from_ref(&v1_utxo.script_pubkey),
            wallet,
        )?;

        Ok(tx)
    }

    pub fn spend_v2_transaction(
        wallet: &mut Wallet,
        to_address: &str,
        recipient_view_pubkey: &RistrettoPoint,
        v2_inputs: &[(OutPoint, u64, BlindingFactor, Vec<u8>)],
        payment_amount: u64,
        fee: u64,
        seed: &[u8; 64],
    ) -> Result<TransactionV2, String> {
        if payment_amount == 0 {
            return Err("Amount must be positive".to_string());
        }
        if v2_inputs.is_empty() {
            return Err("No v2 inputs".to_string());
        }

        let mut total_in: u64 = 0;
        let mut sum_x_in = Scalar::from(0u64);
        let mut inputs = Vec::with_capacity(v2_inputs.len());
        let mut spent_scripts = Vec::with_capacity(v2_inputs.len());
        for (op, value, blind, script) in v2_inputs {
            total_in = total_in
                .checked_add(*value)
                .ok_or_else(|| "Input value overflow".to_string())?;
            sum_x_in += Scalar::from_bytes_mod_order(blind.0);
            inputs.push(TxInputV2 {
                prev_txid: op.txid,
                prev_index: op.vout,
                script_sig: Vec::new(),
                sequence: 0xFFFF_FFFF,
            });
            spent_scripts.push(script.clone());
        }

        let spendable = payment_amount
            .checked_add(fee)
            .ok_or_else(|| "Amount overflow".to_string())?;
        let change = total_in
            .checked_sub(spendable)
            .ok_or_else(|| "Insufficient funds".to_string())?;

        let pay_op = v2_inputs[0].0;
        let pay_blind = derive_blinding(seed, &pay_op);
        let pay_blind_scalar = Scalar::from_bytes_mod_order(pay_blind.0);
        let change_blind = BlindingFactor((sum_x_in - pay_blind_scalar).to_bytes());

        let to_script = address_to_script_pubkey(to_address)?;
        let pay_commitment = commit(payment_amount, &pay_blind);
        let pay_proof = generate_range_proof(payment_amount, &pay_blind)
            .map_err(|e| format!("Range proof (payment): {:?}", e))?;
        let (pay_enc, pay_eph) = encrypt_amount(payment_amount, &pay_blind, recipient_view_pubkey);

        let change_address = wallet.get_or_create_change_address()?;
        let change_script = address_to_script_pubkey(&change_address)?;
        let change_commitment = commit(change, &change_blind);
        let change_proof = generate_range_proof(change, &change_blind)
            .map_err(|e| format!("Range proof (change): {:?}", e))?;
        let (_, own_view_pubkey) = derive_view_key(seed);
        let (change_enc, change_eph) = encrypt_amount(change, &change_blind, &own_view_pubkey);

        let outputs = vec![
            TxOutputV2 {
                commitment: pay_commitment,
                range_proof: pay_proof,
                script_pubkey: to_script,
                encrypted_amount: pay_enc,
                ephemeral_pubkey: pay_eph,
            },
            TxOutputV2 {
                commitment: change_commitment,
                range_proof: change_proof,
                script_pubkey: change_script,
                encrypted_amount: change_enc,
                ephemeral_pubkey: change_eph,
            },
        ];

        let mut tx = TransactionV2 {
            version: TX_VERSION_V2,
            inputs,
            outputs,
            fee,
            locktime: 0,
        };

        sign_v2_transaction(&mut tx, &spent_scripts, wallet)?;

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

fn random_balancing_scalar() -> Scalar {
    let mut wide = [0u8; 64];
    OsRng.fill_bytes(&mut wide);
    Scalar::from_bytes_mod_order_wide(&wide)
}

/// Push `data` onto a script using the minimal valid push opcode.
/// For Dilithium: sig (3309 B) and pubkey (1952 B) both require OP_PUSHDATA2.
fn push_data(script: &mut Vec<u8>, data: &[u8]) {
    if data.len() <= 75 {
        script.push(data.len() as u8);
    } else if data.len() <= 0xFFFF {
        script.push(0x4d); // OP_PUSHDATA2
        script.extend_from_slice(&(data.len() as u16).to_le_bytes());
    }
    script.extend_from_slice(data);
}

fn sign_transaction(
    tx: &mut Transaction,
    spent_outputs: &[TxOutput],
    wallet: &Wallet,
) -> Result<(), String> {
    if tx.inputs.is_empty() {
        return Ok(());
    }

    let mut script_sigs: Vec<Vec<u8>> = Vec::with_capacity(tx.inputs.len());

    for (index, _) in tx.inputs.iter().enumerate() {
        let spent_output = &spent_outputs[index];

        let address = wallet
            .find_address_for_script(&spent_output.script_pubkey)
            .ok_or_else(|| "Address not in wallet".to_string())?;

        let private_key = wallet
            .get_private_key(&address)
            .ok_or_else(|| "Private key not found".to_string())?;

        let sk_bytes = private_key.key_bytes();
        let dilithium_kp = DilithiumKeypair::from_signing_key_bytes(&sk_bytes)
            .map_err(|e| format!("Dilithium key error: {}", e))?;

        let sighash = compute_sighash(tx, index, spent_outputs)
            .map_err(|e| format!("Sighash error: {:?}", e))?;

        let sig_bytes = dilithium_kp.sign(&sighash);
        let pubkey_bytes = dilithium_kp.verifying_key_bytes();

        let mut script_sig = Vec::new();
        push_data(&mut script_sig, &sig_bytes);
        push_data(&mut script_sig, &pubkey_bytes);
        script_sigs.push(script_sig);
    }

    for (index, script_sig) in script_sigs.into_iter().enumerate() {
        tx.inputs[index].script_sig = script_sig;
    }

    Ok(())
}

fn sign_v2_transaction(
    tx: &mut TransactionV2,
    spent_scripts: &[Vec<u8>],
    wallet: &Wallet,
) -> Result<(), String> {
    let mut script_sigs: Vec<Vec<u8>> = Vec::with_capacity(tx.inputs.len());

    for (index, _) in tx.inputs.iter().enumerate() {
        let spent_script = &spent_scripts[index];

        let address = wallet
            .find_address_for_script(spent_script)
            .ok_or_else(|| "Address not in wallet".to_string())?;

        let private_key = wallet
            .get_private_key(&address)
            .ok_or_else(|| "Private key not found".to_string())?;

        let sk_bytes = private_key.key_bytes();
        let dilithium_kp = DilithiumKeypair::from_signing_key_bytes(&sk_bytes)
            .map_err(|e| format!("Dilithium key error: {}", e))?;

        let sighash = compute_sighash_v2(tx, index, spent_script)
            .map_err(|e| format!("Sighash error: {:?}", e))?;

        let sig_bytes = dilithium_kp.sign(&sighash);
        let pubkey_bytes = dilithium_kp.verifying_key_bytes();

        let mut script_sig = Vec::new();
        push_data(&mut script_sig, &sig_bytes);
        push_data(&mut script_sig, &pubkey_bytes);
        script_sigs.push(script_sig);
    }

    for (index, script_sig) in script_sigs.into_iter().enumerate() {
        tx.inputs[index].script_sig = script_sig;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::block::{Block, BlockHeader};
    use crate::consensus::params::Network;
    use crate::consensus::transaction::TxKind;
    use crate::consensus::utxo::{UtxoEntry, UtxoEntryV2};
    use crate::consensus::validation::validate_transaction_v2;
    use crate::crypto::commitments::verify_balance;
    use crate::wallet::keys::derive_view_key;
    use tempfile::TempDir;

    #[test]
    fn test_v2_builder_balance() {
        let dir = TempDir::new().unwrap();
        let wallet_path = dir.path().join("wallet.dat");
        let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();
        wallet.set_seed([0x11u8; 32]).unwrap();
        let addr = wallet.generate_address().unwrap();
        let script = address_to_script_pubkey(&addr).unwrap();

        let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
        let utxo_value = 1_000_000u64;
        let in_op = OutPoint {
            txid: [3u8; 32],
            vout: 0,
        };
        chain
            .utxo_store
            .put_utxo(
                &in_op,
                &UtxoEntry {
                    output: TxOutput {
                        value: utxo_value,
                        script_pubkey: script.clone(),
                    },
                    coinbase: false,
                    height: 0,
                },
            )
            .unwrap();

        let (_, recipient_view_pubkey) = derive_view_key(&[0x22u8; 64]);

        let amount = 400_000u64;
        let fee = 1_000u64;
        let spent = HashSet::new();

        let tx = TransactionBuilder::build_v2_transaction(
            &mut wallet,
            &chain,
            &addr,
            &recipient_view_pubkey,
            amount,
            fee,
            &spent,
        )
        .unwrap();

        assert_eq!(tx.inputs.len(), 1);
        assert_eq!(tx.outputs.len(), 2, "payment + change");
        assert!(!tx.inputs[0].script_sig.is_empty(), "input must be signed");

        let input_commitments = vec![commit(utxo_value, &BlindingFactor([0u8; 32]))];
        let output_commitments: Vec<_> = tx.outputs.iter().map(|o| o.commitment.clone()).collect();
        assert!(verify_balance(&input_commitments, &output_commitments, fee));
    }

    #[test]
    fn test_v2_exact_value_send_blinding_nonzero() {
        let dir = TempDir::new().unwrap();
        let wallet_path = dir.path().join("wallet.dat");
        let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();
        wallet.set_seed([0x11u8; 32]).unwrap();
        let addr = wallet.generate_address().unwrap();
        let script = address_to_script_pubkey(&addr).unwrap();

        let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
        let amount = 400_000u64;
        let fee = 1_000u64;
        let utxo_value = amount + fee;
        let in_op = OutPoint {
            txid: [4u8; 32],
            vout: 0,
        };
        chain
            .utxo_store
            .put_utxo(
                &in_op,
                &UtxoEntry {
                    output: TxOutput {
                        value: utxo_value,
                        script_pubkey: script.clone(),
                    },
                    coinbase: false,
                    height: 0,
                },
            )
            .unwrap();

        let (_, recipient_view_pubkey) = derive_view_key(&[0x22u8; 64]);
        let spent = HashSet::new();

        let tx = TransactionBuilder::build_v2_transaction(
            &mut wallet,
            &chain,
            &addr,
            &recipient_view_pubkey,
            amount,
            fee,
            &spent,
        )
        .unwrap();

        assert_eq!(
            tx.outputs.len(),
            2,
            "exact-value send must still emit a change output"
        );

        let naive = commit(amount, &BlindingFactor([0u8; 32]));
        assert_ne!(
            tx.outputs[0].commitment, naive,
            "payment commitment must not equal value·G — blinding must be non-zero"
        );

        let input_commitments = vec![commit(utxo_value, &BlindingFactor([0u8; 32]))];
        let output_commitments: Vec<_> = tx.outputs.iter().map(|o| o.commitment.clone()).collect();
        assert!(verify_balance(&input_commitments, &output_commitments, fee));
    }

    fn funding_setup() -> (Wallet, ChainState, String, OutPoint, u64, TempDir) {
        let dir = TempDir::new().unwrap();
        let wallet_path = dir.path().join("wallet.dat");
        let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();
        wallet.set_seed([0x33u8; 32]).unwrap();
        let addr = wallet.generate_address().unwrap();
        let script = address_to_script_pubkey(&addr).unwrap();

        let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
        let utxo_value = 1_000_000u64;
        let in_op = OutPoint {
            txid: [7u8; 32],
            vout: 0,
        };
        chain
            .utxo_store
            .put_utxo(
                &in_op,
                &UtxoEntry {
                    output: TxOutput {
                        value: utxo_value,
                        script_pubkey: script,
                    },
                    coinbase: false,
                    height: 0,
                },
            )
            .unwrap();

        (wallet, chain, addr, in_op, utxo_value, dir)
    }

    #[test]
    fn test_v1_to_v2_funding_validates() {
        let (mut wallet, chain, addr, _in_op, _value, _dir) = funding_setup();

        let utxos = wallet.get_utxos(&chain, &HashSet::new()).unwrap();
        assert_eq!(utxos.len(), 1);
        let v1_utxo = utxos[0].clone();

        let (_, recipient_view_pubkey) = derive_view_key(&[0x44u8; 64]);
        let fee = 1_000u64;

        let tx = TransactionBuilder::build_funding_tx(
            &mut wallet,
            &addr,
            &recipient_view_pubkey,
            &v1_utxo,
            fee,
        )
        .unwrap();

        assert_eq!(tx.inputs.len(), 1);
        assert_eq!(tx.outputs.len(), 1);
        assert!(!tx.inputs[0].script_sig.is_empty());

        validate_transaction_v2(&tx, &chain).unwrap();
    }

    #[test]
    fn test_v2_utxo_populated_after_funding() {
        let (mut wallet, chain, addr, in_op, _value, _dir) = funding_setup();

        let v1_utxo = wallet.get_utxos(&chain, &HashSet::new()).unwrap()[0].clone();
        let (_, recipient_view_pubkey) = derive_view_key(&[0x44u8; 64]);
        let fee = 1_000u64;

        let tx = TransactionBuilder::build_funding_tx(
            &mut wallet,
            &addr,
            &recipient_view_pubkey,
            &v1_utxo,
            fee,
        )
        .unwrap();
        validate_transaction_v2(&tx, &chain).unwrap();

        let out_op = OutPoint {
            txid: tx.txid(),
            vout: 0,
        };

        let block = Block {
            header: BlockHeader {
                version: 1,
                prev_block_hash: [0u8; 32],
                merkle_root: [0u8; 32],
                timestamp: 0,
                n_bits: 0,
                nonce: 0,
            },
            transactions: vec![TxKind::V2(tx)],
        };

        let changes = chain.collect_v2_utxo_changes(&block, 1).unwrap();
        assert_eq!(changes.created.len(), 1);
        assert!(changes.spent.is_empty());
        assert_eq!(changes.spent_v1, vec![in_op]);

        chain
            .utxo_store_v2
            .apply_block(&changes.created, &changes.spent)
            .unwrap();
        chain
            .utxo_store
            .apply_block(&[], &changes.spent_v1)
            .unwrap();

        assert!(chain.utxo_store_v2.get_utxo(&out_op).unwrap().is_some());
        assert!(chain.utxo_store.get_utxo(&in_op).unwrap().is_none());
    }

    #[test]
    fn test_v2_spend_v2_utxo() {
        let dir = TempDir::new().unwrap();
        let wallet_path = dir.path().join("wallet.dat");
        let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();
        wallet.set_seed([0x55u8; 32]).unwrap();
        let addr = wallet.generate_address().unwrap();
        let script = address_to_script_pubkey(&addr).unwrap();
        let seed = wallet.bip39_seed().unwrap();

        let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
        let v1_value = 1_000_000u64;
        let v1_op = OutPoint {
            txid: [7u8; 32],
            vout: 0,
        };
        chain
            .utxo_store
            .put_utxo(
                &v1_op,
                &UtxoEntry {
                    output: TxOutput {
                        value: v1_value,
                        script_pubkey: script.clone(),
                    },
                    coinbase: false,
                    height: 0,
                },
            )
            .unwrap();

        let (_, own_view_pubkey) = derive_view_key(&seed);
        let fund_fee = 1_000u64;
        let v1_utxo = wallet.get_utxos(&chain, &HashSet::new()).unwrap()[0].clone();
        let funding = TransactionBuilder::build_funding_tx(
            &mut wallet,
            &addr,
            &own_view_pubkey,
            &v1_utxo,
            fund_fee,
        )
        .unwrap();
        validate_transaction_v2(&funding, &chain).unwrap();

        let fund_block = Block {
            header: BlockHeader {
                version: 1,
                prev_block_hash: [0u8; 32],
                merkle_root: [0u8; 32],
                timestamp: 0,
                n_bits: 0,
                nonce: 0,
            },
            transactions: vec![TxKind::V2(funding.clone())],
        };
        let changes = chain.collect_v2_utxo_changes(&fund_block, 1).unwrap();
        chain
            .utxo_store_v2
            .apply_block(&changes.created, &changes.spent)
            .unwrap();
        chain
            .utxo_store
            .apply_block(&[], &changes.spent_v1)
            .unwrap();

        let v2_value = v1_value - fund_fee;
        let v2_op = OutPoint {
            txid: funding.txid(),
            vout: 0,
        };
        let v2_blind = BlindingFactor([0u8; 32]);

        let (_, recipient_view) = derive_view_key(&[0x66u8; 64]);
        let spend = TransactionBuilder::spend_v2_transaction(
            &mut wallet,
            &addr,
            &recipient_view,
            &[(v2_op, v2_value, v2_blind, script.clone())],
            200_000,
            500,
            &seed,
        )
        .unwrap();

        assert_eq!(spend.inputs.len(), 1);
        assert_eq!(spend.outputs.len(), 2);
        assert!(!spend.inputs[0].script_sig.is_empty());

        validate_transaction_v2(&spend, &chain).unwrap();
    }

    #[test]
    fn test_v2_spend_balance_correct() {
        let dir = TempDir::new().unwrap();
        let wallet_path = dir.path().join("wallet.dat");
        let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();
        wallet.set_seed([0x99u8; 32]).unwrap();
        let addr = wallet.generate_address().unwrap();
        let script = address_to_script_pubkey(&addr).unwrap();
        let seed = wallet.bip39_seed().unwrap();

        let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();

        let v2_value = 500_000u64;
        let x_in = BlindingFactor([9u8; 32]);
        let c_in = commit(v2_value, &x_in);
        let v2_op = OutPoint {
            txid: [3u8; 32],
            vout: 0,
        };
        chain
            .utxo_store_v2
            .put_utxo(
                &v2_op,
                &UtxoEntryV2 {
                    commitment: *c_in.0.as_bytes(),
                    script_pubkey: script.clone(),
                    encrypted_amount: vec![],
                    ephemeral_pubkey: [0u8; 32],
                    coinbase: false,
                    height: 0,
                },
            )
            .unwrap();

        let payment = 100_000u64;
        let fee = 1_000u64;
        let (_, recipient_view) = derive_view_key(&[0x77u8; 64]);
        let spend = TransactionBuilder::spend_v2_transaction(
            &mut wallet,
            &addr,
            &recipient_view,
            &[(v2_op, v2_value, x_in, script.clone())],
            payment,
            fee,
            &seed,
        )
        .unwrap();

        let input_commitments = vec![commit(v2_value, &x_in)];
        let output_commitments: Vec<_> =
            spend.outputs.iter().map(|o| o.commitment.clone()).collect();
        assert!(verify_balance(&input_commitments, &output_commitments, fee));

        validate_transaction_v2(&spend, &chain).unwrap();
    }
}
