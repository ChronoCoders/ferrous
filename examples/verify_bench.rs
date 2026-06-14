use ferrous_node::consensus::transaction::{Transaction, TxInput, TxOutput};
use ferrous_node::consensus::validation::MAX_BLOCK_WEIGHT;
use ferrous_node::script::engine::{validate_p2dl, ScriptContext};
use ferrous_node::script::sighash::compute_sighash;
use ferrous_node::wallet::dilithium::{self, DilithiumKeypair};
use std::hint::black_box;
use std::time::Instant;

use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::VartimeMultiscalarMul;
use sha2::{Digest, Sha512};

use bulletproofs::{BulletproofGens, PedersenGens, RangeProof};
use curve25519_dalek_ng::scalar::Scalar as NgScalar;
use merlin::Transcript;

const RING_SIZE: usize = 11;

fn bench<F: FnMut()>(iters: u32, mut f: F) -> f64 {
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    start.elapsed().as_secs_f64() / iters as f64
}

fn p2dl_spk(pubkey: &[u8]) -> Vec<u8> {
    let h: [u8; 32] = blake3::hash(pubkey).into();
    let mut s = vec![0xaa, 0x20];
    s.extend_from_slice(&h);
    s.push(0x88);
    s.push(0xac);
    s
}

fn p2dl_sig(sig: &[u8], pubkey: &[u8]) -> Vec<u8> {
    let mut s = Vec::new();
    s.push(0x4d);
    s.extend_from_slice(&(sig.len() as u16).to_le_bytes());
    s.extend_from_slice(sig);
    s.push(0x4d);
    s.extend_from_slice(&(pubkey.len() as u16).to_le_bytes());
    s.extend_from_slice(pubkey);
    s
}

fn build_v1_tx(n: usize) -> (Transaction, Vec<TxOutput>, Vec<Vec<u8>>) {
    let mut inputs = Vec::new();
    let mut spent = Vec::new();
    let mut kps = Vec::new();
    let mut pubkeys = Vec::new();
    for i in 0..n {
        let kp = DilithiumKeypair::generate();
        let pk = kp.verifying_key_bytes();
        let spk = p2dl_spk(&pk);
        inputs.push(TxInput {
            prev_txid: [i as u8; 32],
            prev_index: 0,
            script_sig: Vec::new(),
            sequence: 0xFFFF_FFFF,
        });
        spent.push(TxOutput {
            value: 1_000_000,
            script_pubkey: spk,
        });
        kps.push(kp);
        pubkeys.push(pk);
    }
    let outputs = vec![
        TxOutput {
            value: 500_000,
            script_pubkey: spent[0].script_pubkey.clone(),
        },
        TxOutput {
            value: 400_000,
            script_pubkey: spent[0].script_pubkey.clone(),
        },
    ];
    let mut tx = Transaction {
        version: 1,
        inputs,
        outputs,
        witnesses: Vec::new(),
        locktime: 0,
    };
    let mut sigs = Vec::new();
    for i in 0..n {
        let sh = compute_sighash(&tx, i, &spent).unwrap();
        let sig = kps[i].sign(&sh);
        sigs.push(p2dl_sig(&sig, &pubkeys[i]));
    }
    for (input, sig) in tx.inputs.iter_mut().zip(sigs.iter()) {
        input.script_sig = sig.clone();
    }
    (tx, spent, sigs)
}

fn verify_v1_input(tx: &Transaction, spent: &[TxOutput], sigs: &[Vec<u8>], i: usize) -> bool {
    let ctx = ScriptContext {
        transaction: tx,
        input_index: i,
        spent_outputs: spent,
    };
    matches!(
        validate_p2dl(&sigs[i], &spent[i].script_pubkey, &ctx),
        Ok(true)
    )
}

fn challenge_hash(data: &[u8]) -> Scalar {
    let mut h = Sha512::new();
    h.update(data);
    let d = h.finalize();
    let mut wide = [0u8; 64];
    wide.copy_from_slice(&d);
    Scalar::from_bytes_mod_order_wide(&wide)
}

fn main() {
    println!("=== Ferrous verification-cost benchmark (Phase 5 prerequisite) ===");
    println!();

    let mut rng = rand::thread_rng();

    println!("--- v1 (current: Dilithium P2DL) ---");

    let (tx1, spent1, sigs1) = build_v1_tx(1);
    assert!(verify_v1_input(&tx1, &spent1, &sigs1, 0));

    let sample_pk = {
        let kp = DilithiumKeypair::generate();
        let pk = kp.verifying_key_bytes();
        let sh = compute_sighash(&tx1, 0, &spent1).unwrap();
        let sig = kp.sign(&sh);
        (pk, sh, sig)
    };
    let t_dil_verify = bench(300, || {
        black_box(dilithium::verify(&sample_pk.0, &sample_pk.1, &sample_pk.2).ok());
    });
    println!(
        "raw Dilithium verify         : {:.4} ms",
        t_dil_verify * 1e3
    );

    let t_v1_in1 = bench(200, || {
        black_box(verify_v1_input(&tx1, &spent1, &sigs1, 0));
    });
    println!(
        "v1 tx verify (1 input)       : {:.4} ms  ({:.1} tx/s)",
        t_v1_in1 * 1e3,
        1.0 / t_v1_in1
    );

    let (tx10, spent10, sigs10) = build_v1_tx(10);
    for i in 0..10 {
        assert!(verify_v1_input(&tx10, &spent10, &sigs10, i));
    }
    let t_v1_in10 = bench(30, || {
        for i in 0..10 {
            black_box(verify_v1_input(&tx10, &spent10, &sigs10, i));
        }
    });
    println!(
        "v1 tx verify (10 input)      : {:.4} ms  ({:.1} tx/s)",
        t_v1_in10 * 1e3,
        1.0 / t_v1_in10
    );
    println!(
        "v1 throughput (per input)    : {:.1} sig-verify/s",
        1.0 / t_dil_verify
    );

    let base1 = tx1.encode_without_witness().len() as u64;
    let total1 = tx1.encode_with_witness().len() as u64;
    let weight1 = base1 * 3 + total1;
    let v1_tx_per_block = MAX_BLOCK_WEIGHT / weight1;
    println!(
        "v1 tx size (1in/2out)        : base={} B total={} B weight={} units",
        base1, total1, weight1
    );
    println!(
        "v1 tx/block by SIZE @40M     : {} tx   verify time/block: {:.2} s",
        v1_tx_per_block,
        v1_tx_per_block as f64 * t_v1_in1
    );
    println!();

    println!("--- v2 primitives (Ristretto255 / curve25519-dalek 4) ---");
    let p = RISTRETTO_BASEPOINT_POINT * Scalar::random(&mut rng);
    let q = RISTRETTO_BASEPOINT_POINT * Scalar::random(&mut rng);
    let s1 = Scalar::random(&mut rng);
    let s2 = Scalar::random(&mut rng);

    let t_add = bench(20000, || {
        black_box(black_box(p) + black_box(q));
    });
    println!("Ristretto point add          : {:.5} ms", t_add * 1e3);

    let t_scalar_mul = bench(4000, || {
        black_box(black_box(p) * black_box(s1));
    });
    println!(
        "Ristretto scalar mul (varbase): {:.5} ms",
        t_scalar_mul * 1e3
    );

    let t_basemul = bench(4000, || {
        black_box(RISTRETTO_BASEPOINT_POINT * black_box(s1));
    });
    println!("Ristretto basepoint mul      : {:.5} ms", t_basemul * 1e3);

    let scalars = [s1, s2];
    let points = [p, q];
    let t_multiexp2 = bench(4000, || {
        black_box(RistrettoPoint::vartime_multiscalar_mul(&scalars, &points));
    });
    println!("Ristretto multiexp (size 2)  : {:.5} ms", t_multiexp2 * 1e3);

    let cdata = [0u8; 128];
    let t_chal = bench(8000, || {
        black_box(challenge_hash(black_box(&cdata)));
    });
    println!("challenge hash (Sha512→scalar): {:.5} ms", t_chal * 1e3);
    println!();

    println!("--- v2 primitives (Bulletproofs range proof) ---");
    let pc_gens = PedersenGens::default();
    let bp_gens1 = BulletproofGens::new(64, 1);
    let blind1 = NgScalar::random(&mut rng);
    let mut pt1 = Transcript::new(b"verify_bench_rp");
    let (proof1, commit1) =
        RangeProof::prove_single(&bp_gens1, &pc_gens, &mut pt1, 1_037_578_891u64, &blind1, 64)
            .expect("prove_single");
    {
        let mut vt = Transcript::new(b"verify_bench_rp");
        proof1
            .verify_single(&bp_gens1, &pc_gens, &mut vt, &commit1, 64)
            .expect("verify_single");
    }
    let t_bp1 = bench(200, || {
        let mut vt = Transcript::new(b"verify_bench_rp");
        black_box(
            proof1
                .verify_single(&bp_gens1, &pc_gens, &mut vt, &commit1, 64)
                .is_ok(),
        );
    });
    println!("BP range proof verify (m=1)  : {:.4} ms", t_bp1 * 1e3);

    let bp_gens2 = BulletproofGens::new(64, 2);
    let vals = [1_037_578_891u64, 84_321_555u64];
    let blinds = [NgScalar::random(&mut rng), NgScalar::random(&mut rng)];
    let mut pt2 = Transcript::new(b"verify_bench_rp2");
    let (proof2, commits2) =
        RangeProof::prove_multiple(&bp_gens2, &pc_gens, &mut pt2, &vals, &blinds, 64)
            .expect("prove_multiple");
    {
        let mut vt = Transcript::new(b"verify_bench_rp2");
        proof2
            .verify_multiple(&bp_gens2, &pc_gens, &mut vt, &commits2, 64)
            .expect("verify_multiple");
    }
    let t_bp2 = bench(200, || {
        let mut vt = Transcript::new(b"verify_bench_rp2");
        black_box(
            proof2
                .verify_multiple(&bp_gens2, &pc_gens, &mut vt, &commits2, 64)
                .is_ok(),
        );
    });
    println!("BP range proof verify (m=2)  : {:.4} ms", t_bp2 * 1e3);
    println!();

    println!("--- v2 CLSAG verify estimate (ring N={}) ---", RING_SIZE);
    let n = RING_SIZE as f64;
    let clsag_realistic = n * (2.0 * t_multiexp2 + t_chal);
    let clsag_naive = 2.0 * n * t_scalar_mul;
    println!(
        "CLSAG est (realistic, N×(2·multiexp2 + hash)): {:.4} ms",
        clsag_realistic * 1e3
    );
    println!(
        "CLSAG est (lower-bound, 2N·scalar_mul)       : {:.4} ms",
        clsag_naive * 1e3
    );
    println!();

    println!("--- v2 tx verify estimate ---");
    let v2_verify = |n_in: f64, n_out_aggproof: f64, clsag: f64| -> f64 {
        n_in * clsag + n_out_aggproof + t_dil_verify + (n_in + 2.0) * t_add
    };
    let v2_1in2out = v2_verify(1.0, t_bp2, clsag_realistic);
    let v2_2in2out = v2_verify(2.0, t_bp2, clsag_realistic);
    let v2_1in2out_c = v2_verify(1.0, t_bp2, clsag_naive);
    println!(
        "v2 verify 1in/2out (realistic): {:.4} ms  ({:.1} tx/s)",
        v2_1in2out * 1e3,
        1.0 / v2_1in2out
    );
    println!(
        "v2 verify 2in/2out (realistic): {:.4} ms  ({:.1} tx/s)",
        v2_2in2out * 1e3,
        1.0 / v2_2in2out
    );
    println!(
        "v2 verify 1in/2out (low-bnd)  : {:.4} ms  ({:.1} tx/s)",
        v2_1in2out_c * 1e3,
        1.0 / v2_1in2out_c
    );
    println!();

    println!("--- verification budget (v2 1in/2out, realistic) ---");
    for budget in [1.0f64, 5.0, 15.0, 150.0] {
        println!(
            "  {:>4.0} s budget : {:>8.0} tx verified",
            budget,
            budget / v2_1in2out
        );
    }
    println!();

    println!("--- block policy implication ---");
    println!(
        "current MAX_BLOCK_WEIGHT = {} units (v1: {} tx/block, ~{:.1}s verify)",
        MAX_BLOCK_WEIGHT,
        v1_tx_per_block,
        v1_tx_per_block as f64 * t_v1_in1
    );
    for size in [8192u64, 12288, 15360] {
        let w4 = 4 * size;
        let w1 = size;
        let by_size_w4 = MAX_BLOCK_WEIGHT / w4;
        let by_size_w1 = MAX_BLOCK_WEIGHT / w1;
        let verify_full = by_size_w4 as f64 * v2_1in2out;
        println!(
            "  v2 size {:>5} B: by-size {:>4}..{:>4} tx/block (weight=4S..1S); verifying {} tx = {:.1} s",
            size, by_size_w4, by_size_w1, by_size_w4, verify_full
        );
    }
    let budget_s = 15.0f64;
    let verify_bound = (budget_s / v2_1in2out).floor() as u64;
    let weight_for_bound_12k = verify_bound * 4 * 12288;
    println!(
        "  verification-bound @15s budget: {} v2 tx/block",
        verify_bound
    );
    println!(
        "  → that many 12KB tx = {} weight units (vs current {})",
        weight_for_bound_12k, MAX_BLOCK_WEIGHT
    );
    println!();
    println!("=== done ===");
}
