use crate::consensus::transaction::{Transaction, TxOutput, Witness};
use crate::script::opcodes::OpCode;
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
    // 1. Check script size
    if script.len() > MAX_SCRIPT_SIZE {
        return Err(ScriptError::ScriptTooLarge);
    }

    // 2. Parse and execute opcodes
    let mut stack = initial_stack;
    let mut pc = 0; // program counter

    while pc < script.len() {
        if stack.len() > MAX_STACK_SIZE {
            return Err(ScriptError::StackOverflow);
        }

        let op_byte = script[pc];
        pc += 1;

        if op_byte <= 0x4b {
            // Direct push 1-75 bytes
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
            Some(OpCode::OP_1NEGATE) => stack.push(vec![0x81]), // -1
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
                let valid = verify_signature(&pubkey, &signature, context)?;
                if valid {
                    stack.push(vec![1]);
                } else {
                    stack.push(vec![]);
                }
            }
            Some(OpCode::OP_CHECKSIGVERIFY) => {
                if stack.len() < 2 {
                    return Err(ScriptError::StackUnderflow);
                }
                let pubkey = stack.pop().unwrap();
                let signature = stack.pop().unwrap();
                let valid = verify_signature(&pubkey, &signature, context)?;
                if !valid {
                    return Err(ScriptError::VerifyFailed);
                }
            }
            Some(OpCode::OP_RETURN) => {
                return Err(ScriptError::ExecutionFailed);
            }
            None => return Err(ScriptError::InvalidOpcode),
        }
    }

    // 3. Return final stack
    if stack.len() > MAX_STACK_SIZE {
        return Err(ScriptError::StackOverflow);
    }
    Ok(stack)
}

use crate::script::sighash::compute_sighash;
use secp256k1::ecdsa::Signature;
use secp256k1::{Message, PublicKey, SECP256K1};

fn verify_signature(
    pubkey_bytes: &[u8],
    signature: &[u8],
    context: &ScriptContext,
) -> Result<bool, ScriptError> {
    // 1. Parse pubkey (33 bytes compressed secp256k1)
    if pubkey_bytes.len() != 33 {
        return Err(ScriptError::InvalidPubkey);
    }

    let pubkey = PublicKey::from_slice(pubkey_bytes).map_err(|_| ScriptError::InvalidPubkey)?;

    // 2. Parse signature (64 bytes Schnorr)
    if signature.len() != 64 {
        return Err(ScriptError::InvalidSignature);
    }

    // 3. Compute sighash using tagged_hash
    let sighash = compute_sighash(
        context.transaction,
        context.input_index,
        context.spent_outputs,
    )
    .map_err(|_| ScriptError::ExecutionFailed)?;

    // Create message from sighash
    let message =
        Message::from_digest_slice(&sighash).map_err(|_| ScriptError::InvalidSignature)?;

    // 4. Verify using secp256k1
    // Note: For now use ECDSA verify as placeholder
    // Real Schnorr verification requires secp256k1 v0.29+ features
    let sig = Signature::from_compact(signature).map_err(|_| ScriptError::InvalidSignature)?;

    match SECP256K1.verify_ecdsa(&message, &sig, &pubkey) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Validate P2PKH script
pub fn validate_p2pkh(
    script_sig: &[u8],
    script_pubkey: &[u8],
    context: &ScriptContext,
) -> Result<bool, ScriptError> {
    // Execute scriptSig
    let stack = execute_script(script_sig, Vec::new(), context)?;

    // Execute scriptPubKey with resulting stack
    let final_stack = execute_script(script_pubkey, stack, context)?;

    // Check result: stack must have exactly 1 item, which is true
    if final_stack.len() != 1 {
        return Ok(false);
    }

    Ok(is_true(&final_stack[0]))
}

/// Validate P2WPKH script
pub fn validate_p2wpkh(
    witness: &Witness,
    script_pubkey: &[u8],
    context: &ScriptContext,
) -> Result<bool, ScriptError> {
    // scriptPubKey must be: OP_0 <20-byte-hash>
    if script_pubkey.len() != 22 {
        return Ok(false);
    }

    if script_pubkey[0] != 0x00 || script_pubkey[1] != 0x14 {
        return Ok(false);
    }

    // Witness must have exactly 2 items: <signature> <pubkey>
    if witness.stack_items.len() != 2 {
        return Ok(false);
    }

    let signature = &witness.stack_items[0];
    let pubkey = &witness.stack_items[1];

    // Verify pubkey hash matches
    let pubkey_hash = &script_pubkey[2..22];
    let computed_hash = hash160(pubkey);

    if computed_hash.as_slice() != pubkey_hash {
        return Ok(false);
    }

    // Verify signature (placeholder for now)
    verify_signature(pubkey, signature, context)
}

fn is_true(item: &[u8]) -> bool {
    // Empty = false
    // All zeros = false
    // Anything else = true
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
