use crate::consensus::transaction::{Transaction, TxOutput};
use crate::script::opcodes::OpCode;
use crate::script::sighash::compute_sighash;
use crate::wallet::dilithium;
use sha2::{Digest, Sha256};

type Stack = Vec<Vec<u8>>;

const MAX_STACK_SIZE: usize = 1000;
const MAX_SCRIPT_SIZE: usize = 10_000;

pub struct ScriptContext<'a> {
    pub transaction: &'a Transaction,
    pub input_index: usize,
    pub spent_outputs: &'a [TxOutput],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    StackUnderflow,
    StackOverflow,
    ScriptTooLarge,
    InvalidOpcode,
    VerifyFailed,
    EqualVerifyFailed,
    InvalidPushSize,
    InvalidSignature,
    InvalidPubkey,
    ExecutionFailed,
    InvalidWitness,
    UnexpectedEof,
}

pub fn execute_script(
    script: &[u8],
    initial_stack: Stack,
    context: &ScriptContext,
) -> Result<Stack, ScriptError> {
    if script.len() > MAX_SCRIPT_SIZE {
        return Err(ScriptError::ScriptTooLarge);
    }

    let mut stack = initial_stack;
    let mut pc = 0;

    while pc < script.len() {
        if stack.len() > MAX_STACK_SIZE {
            return Err(ScriptError::StackOverflow);
        }

        let op_byte = script[pc];
        pc += 1;

        if op_byte <= 0x4b {
            let len = op_byte as usize;
            if pc + len > script.len() {
                return Err(ScriptError::InvalidPushSize);
            }
            stack.push(script[pc..pc + len].to_vec());
            pc += len;
            continue;
        }

        let opcode = OpCode::from_u8(op_byte);

        match opcode {
            Some(OpCode::OP_0) => stack.push(Vec::new()),
            Some(OpCode::OP_1NEGATE) => stack.push(vec![0x81]),
            Some(OpCode::OP_1) => stack.push(vec![1]),
            Some(OpCode::OP_2) => stack.push(vec![2]),
            Some(OpCode::OP_3) => stack.push(vec![3]),
            Some(OpCode::OP_4) => stack.push(vec![4]),
            Some(OpCode::OP_5) => stack.push(vec![5]),
            Some(OpCode::OP_6) => stack.push(vec![6]),
            Some(OpCode::OP_7) => stack.push(vec![7]),
            Some(OpCode::OP_8) => stack.push(vec![8]),
            Some(OpCode::OP_9) => stack.push(vec![9]),
            Some(OpCode::OP_10) => stack.push(vec![10]),
            Some(OpCode::OP_11) => stack.push(vec![11]),
            Some(OpCode::OP_12) => stack.push(vec![12]),
            Some(OpCode::OP_13) => stack.push(vec![13]),
            Some(OpCode::OP_14) => stack.push(vec![14]),
            Some(OpCode::OP_15) => stack.push(vec![15]),
            Some(OpCode::OP_16) => stack.push(vec![16]),

            Some(OpCode::OP_PUSHDATA1) => {
                if pc + 1 > script.len() {
                    return Err(ScriptError::UnexpectedEof);
                }
                let len = script[pc] as usize;
                pc += 1;
                if pc + len > script.len() {
                    return Err(ScriptError::InvalidPushSize);
                }
                stack.push(script[pc..pc + len].to_vec());
                pc += len;
            }
            Some(OpCode::OP_PUSHDATA2) => {
                if pc + 2 > script.len() {
                    return Err(ScriptError::UnexpectedEof);
                }
                let len = u16::from_le_bytes([script[pc], script[pc + 1]]) as usize;
                pc += 2;
                if pc + len > script.len() {
                    return Err(ScriptError::InvalidPushSize);
                }
                stack.push(script[pc..pc + len].to_vec());
                pc += len;
            }
            Some(OpCode::OP_PUSHDATA4) => {
                if pc + 4 > script.len() {
                    return Err(ScriptError::UnexpectedEof);
                }
                let len = u32::from_le_bytes([
                    script[pc],
                    script[pc + 1],
                    script[pc + 2],
                    script[pc + 3],
                ]) as usize;
                pc += 4;
                if pc + len > script.len() {
                    return Err(ScriptError::InvalidPushSize);
                }
                stack.push(script[pc..pc + len].to_vec());
                pc += len;
            }

            Some(OpCode::OP_DUP) => {
                if stack.is_empty() {
                    return Err(ScriptError::StackUnderflow);
                }
                let top = stack.last().unwrap().clone();
                stack.push(top);
            }
            Some(OpCode::OP_DROP) => {
                if stack.pop().is_none() {
                    return Err(ScriptError::StackUnderflow);
                }
            }
            Some(OpCode::OP_HASH160) => {
                if let Some(item) = stack.pop() {
                    let hash = hash160(&item);
                    stack.push(hash.to_vec());
                } else {
                    return Err(ScriptError::StackUnderflow);
                }
            }
            Some(OpCode::OP_HASH256) => {
                if let Some(item) = stack.pop() {
                    let hash: [u8; 32] = blake3::hash(&item).into();
                    stack.push(hash.to_vec());
                } else {
                    return Err(ScriptError::StackUnderflow);
                }
            }
            Some(OpCode::OP_EQUAL) => {
                if stack.len() < 2 {
                    return Err(ScriptError::StackUnderflow);
                }
                let item1 = stack.pop().unwrap();
                let item2 = stack.pop().unwrap();
                if item1 == item2 {
                    stack.push(vec![1]);
                } else {
                    stack.push(vec![]);
                }
            }
            Some(OpCode::OP_EQUALVERIFY) => {
                if stack.len() < 2 {
                    return Err(ScriptError::StackUnderflow);
                }
                let item1 = stack.pop().unwrap();
                let item2 = stack.pop().unwrap();
                if item1 != item2 {
                    return Err(ScriptError::EqualVerifyFailed);
                }
            }
            Some(OpCode::OP_VERIFY) => {
                if let Some(item) = stack.pop() {
                    if !is_true(&item) {
                        return Err(ScriptError::VerifyFailed);
                    }
                } else {
                    return Err(ScriptError::StackUnderflow);
                }
            }
            Some(OpCode::OP_CHECKSIG) => {
                if stack.len() < 2 {
                    return Err(ScriptError::StackUnderflow);
                }
                let pubkey = stack.pop().unwrap();
                let signature = stack.pop().unwrap();
                let sighash = compute_sighash(
                    context.transaction,
                    context.input_index,
                    context.spent_outputs,
                )
                .map_err(|_| ScriptError::ExecutionFailed)?;
                match dilithium::verify(&pubkey, &sighash, &signature) {
                    Ok(()) => stack.push(vec![1]),
                    Err(_) => stack.push(vec![]),
                }
            }
            Some(OpCode::OP_CHECKSIGVERIFY) => {
                if stack.len() < 2 {
                    return Err(ScriptError::StackUnderflow);
                }
                let pubkey = stack.pop().unwrap();
                let signature = stack.pop().unwrap();
                let sighash = compute_sighash(
                    context.transaction,
                    context.input_index,
                    context.spent_outputs,
                )
                .map_err(|_| ScriptError::ExecutionFailed)?;
                if dilithium::verify(&pubkey, &sighash, &signature).is_err() {
                    return Err(ScriptError::VerifyFailed);
                }
            }
            Some(OpCode::OP_RETURN) => {
                return Err(ScriptError::ExecutionFailed);
            }
            None => return Err(ScriptError::InvalidOpcode),
        }
    }

    if stack.len() > MAX_STACK_SIZE {
        return Err(ScriptError::StackOverflow);
    }
    Ok(stack)
}

/// Decode one OP_PUSHDATA2-prefixed blob from `script_sig` at byte offset `pc`.
/// Returns `(data_slice, new_pc)` on success.
fn parse_pushdata2(script_sig: &[u8], pc: usize) -> Result<(&[u8], usize), ScriptError> {
    if pc >= script_sig.len() || script_sig[pc] != 0x4d {
        return Err(ScriptError::InvalidPushSize);
    }
    let pc = pc + 1;
    if pc + 2 > script_sig.len() {
        return Err(ScriptError::UnexpectedEof);
    }
    let len = u16::from_le_bytes([script_sig[pc], script_sig[pc + 1]]) as usize;
    let pc = pc + 2;
    if pc + len > script_sig.len() {
        return Err(ScriptError::InvalidPushSize);
    }
    Ok((&script_sig[pc..pc + len], pc + len))
}

/// Validate a P2DL (Pay-to-Dilithium) input.
///
/// scriptPubKey: `OP_HASH256(0xaa) push32(0x20) <32-byte BLAKE3 hash> OP_EQUALVERIFY(0x88) OP_CHECKSIG(0xac)` = 36 bytes
/// scriptSig:    `OP_PUSHDATA2 <2-byte-len-LE> <3309-byte sig> OP_PUSHDATA2 <2-byte-len-LE> <1952-byte pubkey>`
pub fn validate_p2dl(
    script_sig: &[u8],
    script_pubkey: &[u8],
    context: &ScriptContext,
) -> Result<bool, ScriptError> {
    let sighash = compute_sighash(
        context.transaction,
        context.input_index,
        context.spent_outputs,
    )
    .map_err(|_| ScriptError::ExecutionFailed)?;

    verify_p2dl_signature(script_sig, script_pubkey, &sighash)
}

pub fn verify_p2dl_signature(
    script_sig: &[u8],
    script_pubkey: &[u8],
    sighash: &[u8],
) -> Result<bool, ScriptError> {
    if script_pubkey.len() != 36
        || script_pubkey[0] != 0xaa
        || script_pubkey[1] != 0x20
        || script_pubkey[34] != 0x88
        || script_pubkey[35] != 0xac
    {
        return Ok(false);
    }
    let script_hash: [u8; 32] = script_pubkey[2..34].try_into().unwrap();

    let (sig_bytes, pc) = parse_pushdata2(script_sig, 0)?;
    let (pubkey_bytes, _) = parse_pushdata2(script_sig, pc)?;

    if sig_bytes.len() != 3309 {
        return Err(ScriptError::InvalidSignature);
    }
    if pubkey_bytes.len() != 1952 {
        return Err(ScriptError::InvalidPubkey);
    }

    let computed_hash: [u8; 32] = blake3::hash(pubkey_bytes).into();
    if computed_hash != script_hash {
        return Ok(false);
    }

    match dilithium::verify(pubkey_bytes, sighash, sig_bytes) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

fn is_true(item: &[u8]) -> bool {
    if item.is_empty() {
        return false;
    }
    for byte in item {
        if *byte != 0 {
            return true;
        }
    }
    false
}

fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    let ripemd = ripemd::Ripemd160::digest(sha);
    let mut result = [0u8; 20];
    result.copy_from_slice(&ripemd);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::transaction::{TxInput, TxOutput};
    use crate::wallet::dilithium::DilithiumKeypair;

    fn make_p2dl_script_pubkey(pubkey_bytes: &[u8]) -> Vec<u8> {
        let hash: [u8; 32] = blake3::hash(pubkey_bytes).into();
        let mut s = vec![0xaa, 0x20];
        s.extend_from_slice(&hash);
        s.push(0x88);
        s.push(0xac);
        s
    }

    fn make_script_sig(sig: &[u8], pubkey: &[u8]) -> Vec<u8> {
        let mut s = Vec::new();
        s.push(0x4d);
        s.extend_from_slice(&(sig.len() as u16).to_le_bytes());
        s.extend_from_slice(sig);
        s.push(0x4d);
        s.extend_from_slice(&(pubkey.len() as u16).to_le_bytes());
        s.extend_from_slice(pubkey);
        s
    }

    fn make_tx_and_spent(script_pubkey: Vec<u8>) -> (Transaction, Vec<TxOutput>) {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                prev_txid: [2u8; 32],
                prev_index: 0,
                script_sig: Vec::new(),
                sequence: 0xFFFF_FFFF,
            }],
            outputs: vec![TxOutput {
                value: 900_000,
                script_pubkey: script_pubkey.clone(),
            }],
            witnesses: Vec::new(),
            locktime: 0,
        };
        let spent = vec![TxOutput {
            value: 1_000_000,
            script_pubkey,
        }];
        (tx, spent)
    }

    #[test]
    fn test_valid_p2dl() {
        let kp = DilithiumKeypair::generate();
        let pubkey_bytes = kp.verifying_key_bytes();
        let script_pubkey = make_p2dl_script_pubkey(&pubkey_bytes);
        let (tx, spent) = make_tx_and_spent(script_pubkey.clone());

        let context = ScriptContext {
            transaction: &tx,
            input_index: 0,
            spent_outputs: &spent,
        };

        let sighash = compute_sighash(&tx, 0, &spent).expect("sighash");
        let sig_bytes = kp.sign(&sighash);
        let script_sig = make_script_sig(&sig_bytes, &pubkey_bytes);

        let result = validate_p2dl(&script_sig, &script_pubkey, &context);
        assert_eq!(result, Ok(true));
    }

    #[test]
    fn test_wrong_pubkey_hash() {
        let kp = DilithiumKeypair::generate();
        let pubkey_bytes = kp.verifying_key_bytes();

        // Build script_pubkey with a wrong hash (all zeros)
        let mut script_pubkey = vec![0xaa, 0x20];
        script_pubkey.extend_from_slice(&[0u8; 32]);
        script_pubkey.push(0x88);
        script_pubkey.push(0xac);

        let (tx, spent) = make_tx_and_spent(script_pubkey.clone());
        let context = ScriptContext {
            transaction: &tx,
            input_index: 0,
            spent_outputs: &spent,
        };

        let sighash = compute_sighash(&tx, 0, &spent).expect("sighash");
        let sig_bytes = kp.sign(&sighash);
        let script_sig = make_script_sig(&sig_bytes, &pubkey_bytes);

        // Hash mismatch → Ok(false)
        let result = validate_p2dl(&script_sig, &script_pubkey, &context);
        assert_eq!(result, Ok(false));
    }

    #[test]
    fn test_wrong_signature() {
        let kp = DilithiumKeypair::generate();
        let pubkey_bytes = kp.verifying_key_bytes();
        let script_pubkey = make_p2dl_script_pubkey(&pubkey_bytes);
        let (tx, spent) = make_tx_and_spent(script_pubkey.clone());

        let context = ScriptContext {
            transaction: &tx,
            input_index: 0,
            spent_outputs: &spent,
        };

        // Sign a different message to produce a wrong sig
        let bad_sig = kp.sign(b"wrong message");
        // Pad to 3309 bytes if needed
        let mut bad_sig = bad_sig;
        bad_sig.resize(3309, 0);
        let script_sig = make_script_sig(&bad_sig, &pubkey_bytes);

        let result = validate_p2dl(&script_sig, &script_pubkey, &context);
        assert_eq!(result, Ok(false));
    }

    #[test]
    fn test_malformed_script_sig() {
        let kp = DilithiumKeypair::generate();
        let pubkey_bytes = kp.verifying_key_bytes();
        let script_pubkey = make_p2dl_script_pubkey(&pubkey_bytes);
        let (tx, spent) = make_tx_and_spent(script_pubkey.clone());

        let context = ScriptContext {
            transaction: &tx,
            input_index: 0,
            spent_outputs: &spent,
        };

        // Truncated: just 10 bytes, no valid OP_PUSHDATA2 structure
        let script_sig = vec![0x4d, 0x00, 0x01, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00];

        let result = validate_p2dl(&script_sig, &script_pubkey, &context);
        assert!(result.is_err(), "expected Err for malformed scriptSig");
    }
}
