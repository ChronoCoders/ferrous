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
use rand::{CryptoRng, RngCore};
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
        let pay_blind = payment_blinding_with_rng(&seed, &pay_op, change, &mut OsRng);

        let to_script = address_to_script_pubkey(to_address)?;
        let pay_commitment = commit(amount, &pay_blind);
        let pay_proof = generate_range_proof(amount, &pay_blind)
            .map_err(|e| format!("Range proof (payment): {:?}", e))?;
        let (pay_enc, pay_eph) = encrypt_amount(amount, &pay_blind, recipient_view_pubkey);

        // The inputs here are v1 outputs, which carry no blinding, so their blindings sum to zero.
        let change_blind = change_blinding(&Scalar::zero(), &pay_blind);
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
        let mut inputs = Vec::with_capacity(v2_inputs.len());
        let mut spent_scripts = Vec::with_capacity(v2_inputs.len());
        for (op, value, _blind, script) in v2_inputs {
            total_in = total_in
                .checked_add(*value)
                .ok_or_else(|| "Input value overflow".to_string())?;
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
        let sum_x_in = sum_input_blindings(v2_inputs.iter().map(|(_, _, blind, _)| blind));
        let change_blind = change_blinding(&sum_x_in, &pay_blind);

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

fn random_balancing_scalar<R: RngCore + CryptoRng>(rng: &mut R) -> Scalar {
    let mut wide = [0u8; 64];
    rng.fill_bytes(&mut wide);
    Scalar::from_bytes_mod_order_wide(&wide)
}

/// Blinding for the payment output of a transaction built from v1 inputs. With change, it is
/// derived from the seed. Without change, it is the negation of a fresh random scalar, so the
/// payment blinding is never zero and an exact-value send cannot reveal its amount (the
/// BLOCKING-2 fix, 958c876); the zero-value change output then carries that scalar.
fn payment_blinding_with_rng<R: RngCore + CryptoRng>(
    seed: &[u8; 64],
    pay_op: &OutPoint,
    change: u64,
    rng: &mut R,
) -> BlindingFactor {
    if change > 0 {
        derive_blinding(seed, pay_op)
    } else {
        BlindingFactor((-random_balancing_scalar(rng)).to_bytes())
    }
}

fn sum_input_blindings<'a>(blinds: impl IntoIterator<Item = &'a BlindingFactor>) -> Scalar {
    blinds.into_iter().fold(Scalar::zero(), |sum, blind| {
        sum + Scalar::from_bytes_mod_order(blind.0)
    })
}

/// Blinding that makes the outputs balance the inputs: the sum of the input blindings minus the
/// payment blinding, so input blindings = payment blinding + change blinding.
fn change_blinding(sum_inputs: &Scalar, pay: &BlindingFactor) -> BlindingFactor {
    BlindingFactor((sum_inputs - Scalar::from_bytes_mod_order(pay.0)).to_bytes())
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
    use crate::crypto::commitments::decrypt_amount;
    use crate::crypto::commitments::verify_balance;
    use crate::network::mempool::NetworkMempool;
    use crate::wallet::keys::derive_view_key;
    use rand_chacha::rand_core::SeedableRng;
    use rand_chacha::ChaCha20Rng;
    use std::sync::{Arc, RwLock};
    use tempfile::TempDir;

    // Golden vectors for the change and payment blinding, round 1b. The expected bytes were
    // generated at f86dd65 from the inline code these functions replaced (builder.rs lines
    // 131-135, 143, 249-256 and 274-276), never from the functions themselves.
    const GOLDEN_SEED: [u8; 64] = [
        0x3c, 0x94, 0x39, 0x82, 0x8c, 0xcd, 0x3e, 0xf7, 0xa1, 0xbf, 0x3f, 0x39, 0x7e, 0x95, 0xc0,
        0xdc, 0xfc, 0xd9, 0x82, 0x63, 0x8d, 0xfe, 0x72, 0x06, 0x9c, 0xcb, 0x51, 0xeb, 0x8c, 0x77,
        0x9e, 0xb2, 0xcb, 0xc7, 0xda, 0x7e, 0xa1, 0xe9, 0xb4, 0x78, 0xdf, 0x4a, 0x18, 0xbd, 0xa2,
        0x7b, 0xaa, 0xd5, 0x51, 0xf1, 0x35, 0x0a, 0xec, 0xe7, 0x20, 0x3f, 0xdd, 0x82, 0x93, 0x74,
        0x66, 0x09, 0xae, 0x65,
    ];
    const GOLDEN_TXID_0: [u8; 32] = [
        0x99, 0x3d, 0x59, 0x08, 0x4b, 0x00, 0xda, 0xc3, 0xd4, 0xb4, 0x6f, 0x4b, 0x6d, 0x02, 0x57,
        0x4f, 0x7b, 0x2e, 0xd2, 0xe3, 0xe6, 0x25, 0xcf, 0xa3, 0x27, 0x5e, 0xbd, 0xc6, 0x7d, 0x15,
        0x88, 0x06,
    ];
    const GOLDEN_TXID_1: [u8; 32] = [
        0xff, 0x49, 0x58, 0xbd, 0x54, 0x99, 0xfb, 0x08, 0xba, 0xca, 0x71, 0xfe, 0x85, 0xe9, 0x27,
        0xe8, 0x03, 0x9f, 0x1e, 0x23, 0xe7, 0x25, 0xcb, 0xe5, 0x1c, 0x6a, 0x37, 0x07, 0x15, 0x7b,
        0xec, 0x30,
    ];
    const GOLDEN_TXID_2: [u8; 32] = [
        0x23, 0x2c, 0x85, 0x9e, 0x7d, 0x01, 0xcb, 0x82, 0x25, 0x73, 0xfd, 0x8a, 0xb1, 0x42, 0x64,
        0xe6, 0x70, 0x7b, 0xee, 0x37, 0xfa, 0xf7, 0xe9, 0x07, 0x41, 0xb1, 0x08, 0xbf, 0x2d, 0x43,
        0x80, 0x5b,
    ];
    const GOLDEN_X_A: [u8; 32] = [
        0xe0, 0x2f, 0x75, 0x9a, 0x08, 0x72, 0xe9, 0x7b, 0x57, 0x61, 0x26, 0xe6, 0x8e, 0xd1, 0xc6,
        0x49, 0x60, 0x57, 0x01, 0x1d, 0x2d, 0xd5, 0x13, 0x01, 0xe7, 0xdc, 0x53, 0x1d, 0xf9, 0x90,
        0x36, 0x0b,
    ];
    const GOLDEN_X_B: [u8; 32] = [
        0xfc, 0x68, 0x63, 0xa2, 0xf9, 0xdb, 0xb0, 0xc0, 0xc8, 0xb9, 0xa7, 0xa2, 0x3d, 0xc5, 0xb1,
        0x9e, 0x36, 0xaa, 0x45, 0x56, 0xf1, 0x96, 0x70, 0x3a, 0xa2, 0xa0, 0x48, 0x34, 0x19, 0x22,
        0xe4, 0x07,
    ];
    const GOLDEN_CHANGE_RNG_SEED: [u8; 32] = [
        0x05, 0x36, 0xca, 0xf1, 0xe7, 0x03, 0x65, 0xd3, 0x56, 0x3b, 0xca, 0xb3, 0x64, 0x88, 0xad,
        0xe1, 0x54, 0x6e, 0x24, 0x62, 0x32, 0xe5, 0x4b, 0x36, 0xd9, 0xc4, 0x2f, 0x75, 0x42, 0xaa,
        0x70, 0x4a,
    ];

    fn golden_op(txid: [u8; 32], vout: u32) -> OutPoint {
        OutPoint { txid, vout }
    }

    fn assert_v1_case(change: u64, pay_hex: &str, change_hex: &str, change_commit_hex: &str) {
        let amount = 7_000u64;
        let fee = 1_000u64;
        let mut rng = ChaCha20Rng::from_seed(GOLDEN_CHANGE_RNG_SEED);
        let pay =
            payment_blinding_with_rng(&GOLDEN_SEED, &golden_op(GOLDEN_TXID_0, 0), change, &mut rng);
        let change_blind = change_blinding(&Scalar::zero(), &pay);
        assert_eq!(hex::encode(pay.0), pay_hex);
        assert_eq!(hex::encode(change_blind.0), change_hex);
        let change_commitment = commit(change, &change_blind);
        assert_eq!(
            hex::encode(change_commitment.0.as_bytes()),
            change_commit_hex
        );
        let inputs = [commit(amount + change + fee, &BlindingFactor([0u8; 32]))];
        assert!(verify_balance(
            &inputs,
            &[commit(amount, &pay), change_commitment],
            fee
        ));
    }

    #[test]
    fn golden_v1_change_positive() {
        assert_v1_case(
            5_000,
            "f76d45a9e0e9ca5dcda3b51350a288d72c2a2a6875c9f29b62cfae28312e780f",
            "f665b0b3397947fa08f9418f8e57563dd3d5d5978a360d649d3051d7ced18700",
            "e067d431f6d08e2a9415993065b6160660efa0d3ae947b7517376242f5f71a4f",
        );
    }

    // change == 1 is the smallest value on the derived branch; it pins the branch boundary.
    #[test]
    fn golden_v1_change_one() {
        assert_v1_case(
            1,
            "f76d45a9e0e9ca5dcda3b51350a288d72c2a2a6875c9f29b62cfae28312e780f",
            "f665b0b3397947fa08f9418f8e57563dd3d5d5978a360d649d3051d7ced18700",
            "9c3c659fb53ee543c39fb80c64ef11be23315064ba40f3ff8c8c66e970fd8630",
        );
    }

    #[test]
    fn golden_v1_change_zero() {
        assert_v1_case(
            0,
            "089284592451375cead48fa61eca408337cea7296744a0e91519213a6a1c0106",
            "e5417103f611dbfbebc767fcbf2f9e91c83158d698bb5f16eae6dec595e3fe09",
            "eafbcca8c32d5fb21cd056d1687b2b0c524b7843123068e6d4293e7671e93216",
        );
        let mut rng = ChaCha20Rng::from_seed(GOLDEN_CHANGE_RNG_SEED);
        let pay =
            payment_blinding_with_rng(&GOLDEN_SEED, &golden_op(GOLDEN_TXID_0, 0), 0, &mut rng);
        let change_blind = change_blinding(&Scalar::zero(), &pay);
        assert_eq!(
            Scalar::from_bytes_mod_order(pay.0) + Scalar::from_bytes_mod_order(change_blind.0),
            Scalar::zero()
        );
    }

    fn assert_v2_case(
        inputs: &[(OutPoint, u64, BlindingFactor)],
        payment: u64,
        sum_hex: &str,
        pay_hex: &str,
        change_hex: &str,
        change_commit_hex: &str,
    ) {
        let fee = 1_000u64;
        let total: u64 = inputs.iter().map(|(_, value, _)| value).sum();
        let change = total - payment - fee;
        let sum = sum_input_blindings(inputs.iter().map(|(_, _, blind)| blind));
        let pay = derive_blinding(&GOLDEN_SEED, &inputs[0].0);
        let change_blind = change_blinding(&sum, &pay);
        assert_eq!(hex::encode(sum.to_bytes()), sum_hex);
        assert_eq!(hex::encode(pay.0), pay_hex);
        assert_eq!(hex::encode(change_blind.0), change_hex);
        let change_commitment = commit(change, &change_blind);
        assert_eq!(
            hex::encode(change_commitment.0.as_bytes()),
            change_commit_hex
        );
        let input_commitments: Vec<_> = inputs
            .iter()
            .map(|(_, value, blind)| commit(*value, blind))
            .collect();
        assert!(verify_balance(
            &input_commitments,
            &[commit(payment, &pay), change_commitment],
            fee
        ));
    }

    #[test]
    fn golden_v2_one_input() {
        assert_v2_case(
            &[(
                golden_op(GOLDEN_TXID_1, 1),
                20_000,
                BlindingFactor(GOLDEN_X_A),
            )],
            12_000,
            "e02f759a0872e97b576126e68ed1c6496057011d2dd51301e7dc531df990360b",
            "9004b8a79595fc98dc5187ce5586e97287fbdca362abe15e222e8605b1e3570e",
            "3dffb24f8d3fff3a51ac96ba1745bcebd85b2479ca2932a2c4aecd1748adde0c",
            "bc7ae09ea7121a99c639b1b127388e073939fdab4ba837ddf3547299e936715c",
        );
    }

    // The middle input has a zero blinding, as a funding output does.
    #[test]
    fn golden_v2_three_inputs() {
        assert_v2_case(
            &[
                (
                    golden_op(GOLDEN_TXID_2, 2),
                    5_000,
                    BlindingFactor(GOLDEN_X_A),
                ),
                (
                    golden_op(GOLDEN_TXID_0, 5),
                    8_000,
                    BlindingFactor([0u8; 32]),
                ),
                (
                    golden_op(GOLDEN_TXID_1, 6),
                    9_000,
                    BlindingFactor(GOLDEN_X_B),
                ),
            ],
            15_000,
            "efc4e2dfe7ea87e4497ed6e5ed9c99d3960147731e6c843b897d9c5112b31a03",
            "d1b9865787847abd617e4a483c1bcfd88d26d48ca67019cb6eb46d4b6ccca107",
            "0bdf51e57ac91f7fbe9c8340907ba90f09db72e677fb6a701ac92e06a6e6780b",
            "dcfc3016c6252aca620a11fa6b3209f8c2c49841a28a138141f3adf44cd03e02",
        );
    }

    // 32 bytes of 0xff is not a canonical scalar; the sum must reduce it mod l.
    #[test]
    fn golden_v2_noncanonical_input() {
        assert_v2_case(
            &[
                (
                    golden_op(GOLDEN_TXID_0, 3),
                    4_000,
                    BlindingFactor([0xffu8; 32]),
                ),
                (
                    golden_op(GOLDEN_TXID_1, 4),
                    6_000,
                    BlindingFactor(GOLDEN_X_A),
                ),
            ],
            8_000,
            "0ff117cb6240c3faf193acb6a433d7fb5e57011d2dd51301e7dc531df990360b",
            "07b5221c327ce107d0ae9d7474e835b2061f6b3678107402a594dd114e39d307",
            "083cf5ae30c4e1f221e50e42304ba149583896e6b4c49ffe4148760bab576303",
            "ccdcfdc426b342b48545bfd7929e075cdccad131adca041a95aab1282c2b326e",
        );
    }

    #[test]
    fn funding_output_blinding_is_zero() {
        let dir = TempDir::new().unwrap();
        let mut wallet = Wallet::load(dir.path().join("wallet.dat"), 0x6f).unwrap();
        wallet.set_seed([0x55u8; 32]).unwrap();
        let addr = wallet.generate_address().unwrap();
        let v1_utxo = WalletUtxo {
            txid: GOLDEN_TXID_2,
            vout: 0,
            value: 50_000,
            script_pubkey: address_to_script_pubkey(&addr).unwrap(),
            height: 0,
        };
        let (view_scalar, view_pubkey) = derive_view_key(&GOLDEN_SEED);
        let fee = 1_000u64;
        let tx =
            TransactionBuilder::build_funding_tx(&mut wallet, &addr, &view_pubkey, &v1_utxo, fee)
                .unwrap();
        let output = &tx.outputs[0];
        let (value, blinding) = decrypt_amount(
            &output.encrypted_amount,
            &output.ephemeral_pubkey,
            &view_scalar,
        )
        .unwrap();
        assert_eq!(value, 50_000 - fee);
        assert_eq!(blinding.0, [0u8; 32]);
        assert_eq!(output.commitment, commit(value, &BlindingFactor([0u8; 32])));
    }

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
            "payment commitment must not equal value·G: blinding must be non-zero"
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

    #[test]
    fn test_zero_value_v2_change_accepted() {
        let dir = TempDir::new().unwrap();
        let wallet_path = dir.path().join("wallet.dat");
        let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();
        wallet.set_seed([0xABu8; 32]).unwrap();
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

        let fee = 1_000u64;
        let payment = v2_value - fee;
        let (_, recipient_view) = derive_view_key(&[0xCDu8; 64]);
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

        assert_eq!(spend.outputs.len(), 2);
        let pay_blind = derive_blinding(&seed, &v2_op);
        let change_blind = BlindingFactor(
            (Scalar::from_bytes_mod_order(x_in.0) - Scalar::from_bytes_mod_order(pay_blind.0))
                .to_bytes(),
        );
        assert_eq!(spend.outputs[1].commitment, commit(0, &change_blind));

        validate_transaction_v2(&spend, &chain).unwrap();

        let chain = Arc::new(RwLock::new(chain));
        let mempool = NetworkMempool::new(chain);
        let admitted = mempool.add_transaction(TxKind::V2(spend)).unwrap();
        assert!(admitted);
    }

    #[test]
    fn test_funding_tx_immature_coinbase_rejected_mempool() {
        let dir = TempDir::new().unwrap();
        let wallet_path = dir.path().join("wallet.dat");
        let mut wallet = Wallet::load(&wallet_path, 0x6f).unwrap();
        wallet.set_seed([0xBCu8; 32]).unwrap();
        let addr = wallet.generate_address().unwrap();
        let script = address_to_script_pubkey(&addr).unwrap();

        let chain = ChainState::new_in_memory(Network::Regtest.params()).unwrap();
        let cb_value = 50 * 100_000_000u64;
        let cb_op = OutPoint {
            txid: [8u8; 32],
            vout: 0,
        };
        chain
            .utxo_store
            .put_utxo(
                &cb_op,
                &UtxoEntry {
                    output: TxOutput {
                        value: cb_value,
                        script_pubkey: script.clone(),
                    },
                    coinbase: true,
                    height: 0,
                },
            )
            .unwrap();

        let v1_utxo = WalletUtxo {
            txid: cb_op.txid,
            vout: cb_op.vout,
            value: cb_value,
            script_pubkey: script.clone(),
            height: 0,
        };
        let (_, recipient_view) = derive_view_key(&[0xDEu8; 64]);
        let funding = TransactionBuilder::build_funding_tx(
            &mut wallet,
            &addr,
            &recipient_view,
            &v1_utxo,
            1_000,
        )
        .unwrap();

        let chain = Arc::new(RwLock::new(chain));
        let mempool = NetworkMempool::new(chain);
        let res = mempool.add_transaction(TxKind::V2(funding));
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("ImmatureCoinbase"));
    }
}
