/// Finds the genesis block nonce for the current testnet RandomX target.
/// Run once on Linux after changing TESTNET_MAX_TARGET; record the output
/// in the known_nonce table in block.rs.
#[cfg(target_os = "linux")]
fn main() {
    use ferrous_node::consensus::block::BlockHeader;
    use ferrous_node::consensus::merkle::compute_merkle_root;
    use ferrous_node::consensus::transaction::{Transaction, TxInput, TxOutput, Witness};
    use std::time::Instant;

    // Must match create_genesis_block exactly.
    let genesis_n_bits: u32 = 0x1F10_CDD9;

    let coinbase = Transaction {
        version: 1,
        inputs: vec![TxInput {
            prev_txid: [0u8; 32],
            prev_index: 0xFFFF_FFFF,
            script_sig: vec![0x04, 0x00, 0x00, 0x00, 0x00],
            sequence: 0xFFFF_FFFF,
        }],
        outputs: vec![TxOutput {
            value: 50 * 100_000_000,
            script_pubkey: vec![0x51],
        }],
        witnesses: vec![Witness {
            stack_items: Vec::new(),
        }],
        locktime: 0,
    };

    let txids = vec![coinbase.txid()];
    let merkle_root = compute_merkle_root(&txids);

    let mut header = BlockHeader {
        version: 1,
        prev_block_hash: [0u8; 32],
        merkle_root,
        timestamp: 1_700_000_000,
        n_bits: genesis_n_bits,
        nonce: 0,
    };

    let epoch_key = BlockHeader::epoch_key(0);

    println!(
        "Searching for genesis nonce under RandomX (n_bits = {:#010X})...",
        genesis_n_bits
    );
    let start = Instant::now();
    let mut checked = 0u64;

    loop {
        if header.check_proof_of_work(&epoch_key).unwrap_or(false) {
            break;
        }
        header.nonce = header.nonce.wrapping_add(1);
        checked += 1;

        if checked % 100 == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            print!(
                "\r  nonce={} checked={} elapsed={:.0}s rate={:.2} H/s    ",
                header.nonce,
                checked,
                elapsed,
                checked as f64 / elapsed.max(0.001)
            );
            use std::io::Write;
            std::io::stdout().flush().ok();
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    println!();
    println!("Found genesis nonce: {}", header.nonce);
    println!(
        "Checked {} hashes in {:.1}s ({:.2} H/s)",
        checked,
        elapsed,
        checked as f64 / elapsed
    );
    println!();
    println!(
        "Add to block.rs known_nonce table:\n    {:#010X} => Some({}),",
        genesis_n_bits, header.nonce
    );
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("find_genesis_nonce only runs on Linux (RandomX requires Linux). SSH into seed4.");
    std::process::exit(1);
}
