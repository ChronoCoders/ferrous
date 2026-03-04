use ferrous_node::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
use ferrous_node::script::engine::{
    execute_script, validate_p2pkh, validate_p2wpkh, ScriptContext, ScriptError,
};
use ferrous_node::script::opcodes::OpCode;
use ferrous_node::script::sighash::compute_sighash;
use rand::thread_rng;
use secp256k1::{Message, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};

fn sample_tx_input(index: u32) -> TxInput {
    TxInput {
        prev_txid: [0xAA; 32],
        prev_index: index,
        script_sig: vec![],
        sequence: 0xFFFFFFFF,
    }
}

fn sample_tx_output(value: u64, script_len: usize) -> TxOutput {
    let mut script = Vec::with_capacity(script_len);
    for i in 0..script_len {
        script.push(255u8 - (i % 256) as u8);
    }
    TxOutput {
        value,
        script_pubkey: script,
    }
}

fn mock_context_real_sig() -> (Transaction, Vec<TxOutput>) {
    let input = sample_tx_input(0);
    let output = sample_tx_output(100_000, 20);
    let tx = Transaction {
        version: 1,
        inputs: vec![input],
        outputs: vec![output],
        witnesses: vec![Witness {
            stack_items: vec![],
        }],
        locktime: 0,
    };
    let spent_outputs = vec![sample_tx_output(50_000, 25)];
    (tx, spent_outputs)
}

fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    let ripemd = ripemd::Ripemd160::digest(sha);
    let mut result = [0u8; 20];
    result.copy_from_slice(&ripemd);
    result
}

fn create_test_keypair() -> (SecretKey, Vec<u8>) {
    let secp = Secp256k1::new();
    let mut rng = thread_rng();
    let secret_key = SecretKey::new(&mut rng);
    let pubkey = secret_key.public_key(&secp);
    (secret_key, pubkey.serialize().to_vec())
}

fn create_test_signature(
    secret_key: &SecretKey,
    tx: &Transaction,
    input_index: usize,
    spent_outputs: &[TxOutput],
) -> Vec<u8> {
    let secp = Secp256k1::new();
    let sighash = compute_sighash(tx, input_index, spent_outputs).unwrap();
    let message = Message::from_digest_slice(&sighash).unwrap();
    let sig = secp.sign_ecdsa(&message, secret_key);
    sig.serialize_compact().to_vec()
}

#[test]
fn test_opcode_from_u8() {
    assert_eq!(OpCode::from_u8(0x00), Some(OpCode::OP_0));
    assert_eq!(OpCode::from_u8(0x76), Some(OpCode::OP_DUP));
    assert_eq!(OpCode::from_u8(0xac), Some(OpCode::OP_CHECKSIG));
    assert_eq!(OpCode::from_u8(0xff), None);
}

#[test]
fn test_stack_push_ops() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let script = vec![
        0x01,
        0x42, // PUSH 1 byte (0x42)
        0x02,
        0x01,
        0x02,                     // PUSH 2 bytes (0x01, 0x02)
        OpCode::OP_0 as u8,       // PUSH empty
        OpCode::OP_1 as u8,       // PUSH 1
        OpCode::OP_16 as u8,      // PUSH 16
        OpCode::OP_1NEGATE as u8, // PUSH -1
    ];

    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack.len(), 6);
    assert_eq!(stack[0], vec![0x42]);
    assert_eq!(stack[1], vec![0x01, 0x02]);
    assert_eq!(stack[2], Vec::<u8>::new());
    assert_eq!(stack[3], vec![1]);
    assert_eq!(stack[4], vec![16]);
    assert_eq!(stack[5], vec![0x81]);
}

#[test]
fn test_stack_dup_drop() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    // PUSH 1, DUP, DROP
    let script = vec![
        OpCode::OP_1 as u8,
        OpCode::OP_DUP as u8,
        OpCode::OP_DROP as u8,
    ];
    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0], vec![1]);
}

#[test]
fn test_stack_underflow() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let script = vec![OpCode::OP_DROP as u8];
    let err = execute_script(&script, Vec::new(), &context).unwrap_err();
    assert_eq!(err, ScriptError::StackUnderflow);
}

#[test]
fn test_stack_overflow() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    // Create a script that pushes 1001 items
    let mut script = Vec::new();
    for _ in 0..1001 {
        script.push(OpCode::OP_1 as u8);
    }

    let err = execute_script(&script, Vec::new(), &context).unwrap_err();
    assert_eq!(err, ScriptError::StackOverflow);
}

#[test]
fn test_crypto_hash160() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let data = b"test";
    let expected = hash160(data);

    // PUSH "test", HASH160
    let mut script = vec![0x04];
    script.extend_from_slice(data);
    script.push(OpCode::OP_HASH160 as u8);

    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0], expected);
}

#[test]
fn test_op_equal() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    // PUSH 1, PUSH 1, EQUAL
    let script = vec![
        OpCode::OP_1 as u8,
        OpCode::OP_1 as u8,
        OpCode::OP_EQUAL as u8,
    ];
    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0], vec![1]); // True

    // PUSH 1, PUSH 2, EQUAL
    let script = vec![
        OpCode::OP_1 as u8,
        OpCode::OP_2 as u8,
        OpCode::OP_EQUAL as u8,
    ];
    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0], Vec::<u8>::new()); // False
}

#[test]
fn test_op_equalverify() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    // PUSH 1, PUSH 1, EQUALVERIFY (Success)
    let script = vec![
        OpCode::OP_1 as u8,
        OpCode::OP_1 as u8,
        OpCode::OP_EQUALVERIFY as u8,
    ];
    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack.len(), 0);

    // PUSH 1, PUSH 2, EQUALVERIFY (Fail)
    let script = vec![
        OpCode::OP_1 as u8,
        OpCode::OP_2 as u8,
        OpCode::OP_EQUALVERIFY as u8,
    ];
    let err = execute_script(&script, Vec::new(), &context).unwrap_err();
    assert_eq!(err, ScriptError::EqualVerifyFailed);
}

#[test]
fn test_script_too_large() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let script = vec![0u8; 10_001];
    let err = execute_script(&script, Vec::new(), &context).unwrap_err();
    assert_eq!(err, ScriptError::ScriptTooLarge);
}

#[test]
fn test_invalid_opcode() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let script = vec![0xFF];
    let err = execute_script(&script, Vec::new(), &context).unwrap_err();
    assert_eq!(err, ScriptError::InvalidOpcode);
}

#[test]
fn test_p2pkh_validation() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let (secret_key, pubkey) = create_test_keypair();
    let signature = create_test_signature(&secret_key, &tx, 0, &outputs);
    let pubkey_hash = hash160(&pubkey);

    // scriptSig: <sig> <pubkey>
    let mut script_sig = Vec::new();
    script_sig.push(signature.len() as u8);
    script_sig.extend_from_slice(&signature);
    script_sig.push(pubkey.len() as u8);
    script_sig.extend_from_slice(&pubkey);

    // scriptPubKey: OP_DUP OP_HASH160 <pubkey_hash> OP_EQUALVERIFY OP_CHECKSIG
    let mut script_pubkey = Vec::new();
    script_pubkey.push(OpCode::OP_DUP as u8);
    script_pubkey.push(OpCode::OP_HASH160 as u8);
    script_pubkey.push(pubkey_hash.len() as u8);
    script_pubkey.extend_from_slice(&pubkey_hash);
    script_pubkey.push(OpCode::OP_EQUALVERIFY as u8);
    script_pubkey.push(OpCode::OP_CHECKSIG as u8);

    let result = validate_p2pkh(&script_sig, &script_pubkey, &context).unwrap();
    assert!(result);
}

#[test]
fn test_p2wpkh_validation() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let (secret_key, pubkey) = create_test_keypair();
    let signature = create_test_signature(&secret_key, &tx, 0, &outputs);
    let pubkey_hash = hash160(&pubkey);

    // Witness: <sig> <pubkey>
    let witness = Witness {
        stack_items: vec![signature, pubkey],
    };

    // scriptPubKey: OP_0 <20-byte-hash>
    let mut script_pubkey = Vec::new();
    script_pubkey.push(OpCode::OP_0 as u8);
    script_pubkey.push(pubkey_hash.len() as u8);
    script_pubkey.extend_from_slice(&pubkey_hash);

    let result = validate_p2wpkh(&witness, &script_pubkey, &context).unwrap();
    assert!(result);
}

#[test]
fn test_p2wpkh_validation_fail_bad_hash() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let (secret_key, pubkey) = create_test_keypair();
    let signature = create_test_signature(&secret_key, &tx, 0, &outputs);
    let bad_hash = [0x00; 20];

    let witness = Witness {
        stack_items: vec![signature, pubkey],
    };

    let mut script_pubkey = Vec::new();
    script_pubkey.push(OpCode::OP_0 as u8);
    script_pubkey.push(bad_hash.len() as u8);
    script_pubkey.extend_from_slice(&bad_hash);

    let result = validate_p2wpkh(&witness, &script_pubkey, &context).unwrap();
    assert!(!result);
}

#[test]
fn test_pushdata_ops() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    // OP_PUSHDATA1
    let mut script = vec![OpCode::OP_PUSHDATA1 as u8, 0x05];
    script.extend_from_slice(&[1, 2, 3, 4, 5]);

    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack[0], vec![1, 2, 3, 4, 5]);

    // OP_PUSHDATA2 (length 256)
    let mut script = vec![OpCode::OP_PUSHDATA2 as u8, 0x00, 0x01]; // 256 little endian
    let data = vec![0xAA; 256];
    script.extend_from_slice(&data);

    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack[0], data);
}

#[test]
fn test_op_verify() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    // PUSH 1, VERIFY (Success)
    let script = vec![OpCode::OP_1 as u8, OpCode::OP_VERIFY as u8];
    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack.len(), 0);

    // PUSH 0, VERIFY (Fail)
    let script = vec![OpCode::OP_0 as u8, OpCode::OP_VERIFY as u8];
    let err = execute_script(&script, Vec::new(), &context).unwrap_err();
    assert_eq!(err, ScriptError::VerifyFailed);
}

#[test]
fn test_op_return() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let script = vec![OpCode::OP_RETURN as u8];
    let err = execute_script(&script, Vec::new(), &context).unwrap_err();
    assert_eq!(err, ScriptError::ExecutionFailed);
}

#[test]
fn test_checksig_real() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let (secret_key, pubkey) = create_test_keypair();
    let signature = create_test_signature(&secret_key, &tx, 0, &outputs);

    // PUSH sig, PUSH pubkey, CHECKSIG
    let mut script = Vec::new();
    script.push(signature.len() as u8);
    script.extend_from_slice(&signature);
    script.push(pubkey.len() as u8);
    script.extend_from_slice(&pubkey);
    script.push(OpCode::OP_CHECKSIG as u8);

    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack[0], vec![1]);
}

#[test]
fn test_checksig_real_fail() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let (secret_key, pubkey) = create_test_keypair();
    let mut signature = create_test_signature(&secret_key, &tx, 0, &outputs);
    // Corrupt signature
    signature[0] ^= 0xFF;

    // PUSH sig, PUSH pubkey, CHECKSIG
    let mut script = Vec::new();
    script.push(signature.len() as u8);
    script.extend_from_slice(&signature);
    script.push(pubkey.len() as u8);
    script.extend_from_slice(&pubkey);
    script.push(OpCode::OP_CHECKSIG as u8);

    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack[0], Vec::<u8>::new()); // False
}

#[test]
fn test_checksigverify_real() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let (secret_key, pubkey) = create_test_keypair();
    let signature = create_test_signature(&secret_key, &tx, 0, &outputs);

    // PUSH sig, PUSH pubkey, CHECKSIGVERIFY
    let mut script = Vec::new();
    script.push(signature.len() as u8);
    script.extend_from_slice(&signature);
    script.push(pubkey.len() as u8);
    script.extend_from_slice(&pubkey);
    script.push(OpCode::OP_CHECKSIGVERIFY as u8);

    let stack = execute_script(&script, Vec::new(), &context).unwrap();
    assert_eq!(stack.len(), 0);
}

#[test]
fn test_checksigverify_real_fail() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    let (secret_key, pubkey) = create_test_keypair();
    let mut signature = create_test_signature(&secret_key, &tx, 0, &outputs);
    // Corrupt signature
    signature[0] ^= 0xFF;

    // PUSH sig, PUSH pubkey, CHECKSIGVERIFY
    let mut script = Vec::new();
    script.push(signature.len() as u8);
    script.extend_from_slice(&signature);
    script.push(pubkey.len() as u8);
    script.extend_from_slice(&pubkey);
    script.push(OpCode::OP_CHECKSIGVERIFY as u8);

    let err = execute_script(&script, Vec::new(), &context).unwrap_err();
    assert_eq!(err, ScriptError::VerifyFailed);
}

#[test]
fn test_invalid_push_size() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    // PUSH 5 bytes but only provide 1
    let script = vec![0x05, 0x01];
    let err = execute_script(&script, Vec::new(), &context).unwrap_err();
    assert_eq!(err, ScriptError::InvalidPushSize);
}

#[test]
fn test_unexpected_eof_pushdata() {
    let (tx, outputs) = mock_context_real_sig();
    let context = ScriptContext {
        transaction: &tx,
        input_index: 0,
        spent_outputs: &outputs,
    };

    // OP_PUSHDATA1 but no length byte
    let script = vec![OpCode::OP_PUSHDATA1 as u8];
    let err = execute_script(&script, Vec::new(), &context).unwrap_err();
    assert_eq!(err, ScriptError::UnexpectedEof);
}
