/// Script opcodes (subset for P2PKH/P2WPKH support)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(non_camel_case_types)]
pub enum OpCode {
    // Push value
    OP_0 = 0x00,
    OP_PUSHDATA1 = 0x4c,
    OP_PUSHDATA2 = 0x4d,
    OP_PUSHDATA4 = 0x4e,
    OP_1NEGATE = 0x4f,
    OP_1 = 0x51,
    OP_2 = 0x52,
    OP_3 = 0x53,
    OP_4 = 0x54,
    OP_5 = 0x55,
    OP_6 = 0x56,
    OP_7 = 0x57,
    OP_8 = 0x58,
    OP_9 = 0x59,
    OP_10 = 0x5a,
    OP_11 = 0x5b,
    OP_12 = 0x5c,
    OP_13 = 0x5d,
    OP_14 = 0x5e,
    OP_15 = 0x5f,
    OP_16 = 0x60,

    // Stack ops (needed for P2PKH)
    OP_DUP = 0x76,
    OP_DROP = 0x75,

    // Crypto ops
    OP_HASH160 = 0xa9,
    OP_EQUAL = 0x87,
    OP_EQUALVERIFY = 0x88,
    OP_CHECKSIG = 0xac,
    OP_CHECKSIGVERIFY = 0xad,

    // Control
    OP_VERIFY = 0x69,
    OP_RETURN = 0x6a,
}

impl OpCode {
    pub fn from_u8(byte: u8) -> Option<OpCode> {
        match byte {
            0x00 => Some(OpCode::OP_0),
            0x4c => Some(OpCode::OP_PUSHDATA1),
            0x4d => Some(OpCode::OP_PUSHDATA2),
            0x4e => Some(OpCode::OP_PUSHDATA4),
            0x4f => Some(OpCode::OP_1NEGATE),
            0x51 => Some(OpCode::OP_1),
            0x52 => Some(OpCode::OP_2),
            0x53 => Some(OpCode::OP_3),
            0x54 => Some(OpCode::OP_4),
            0x55 => Some(OpCode::OP_5),
            0x56 => Some(OpCode::OP_6),
            0x57 => Some(OpCode::OP_7),
            0x58 => Some(OpCode::OP_8),
            0x59 => Some(OpCode::OP_9),
            0x5a => Some(OpCode::OP_10),
            0x5b => Some(OpCode::OP_11),
            0x5c => Some(OpCode::OP_12),
            0x5d => Some(OpCode::OP_13),
            0x5e => Some(OpCode::OP_14),
            0x5f => Some(OpCode::OP_15),
            0x60 => Some(OpCode::OP_16),
            0x76 => Some(OpCode::OP_DUP),
            0x75 => Some(OpCode::OP_DROP),
            0xa9 => Some(OpCode::OP_HASH160),
            0x87 => Some(OpCode::OP_EQUAL),
            0x88 => Some(OpCode::OP_EQUALVERIFY),
            0xac => Some(OpCode::OP_CHECKSIG),
            0xad => Some(OpCode::OP_CHECKSIGVERIFY),
            0x69 => Some(OpCode::OP_VERIFY),
            0x6a => Some(OpCode::OP_RETURN),
            _ => None,
        }
    }
}
