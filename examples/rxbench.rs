/// Standalone RandomX throughput benchmark.
/// Measures sustained H/s with the thread_local VM — no reinit overhead.
/// Usage: cargo run --release --example rxbench
#[cfg(target_os = "linux")]
fn main() {
    use randomx_rs::{RandomXCache, RandomXFlag, RandomXVM};
    use std::cell::RefCell;
    use std::time::Instant;

    thread_local! {
        static VM: RefCell<Option<RandomXVM>> = RefCell::new(None);
    }

    let epoch_key = [0u8; 32];
    let flags = RandomXFlag::get_recommended_flags();

    VM.with(|cell| {
        let mut slot = cell.borrow_mut();
        let cache = RandomXCache::new(flags, &epoch_key).expect("cache init");
        let vm = RandomXVM::new(flags, Some(cache), None).expect("vm init");
        *slot = Some(vm);
    });

    println!("Warming up 3 s...");
    let warmup = Instant::now();
    let mut input = [0u8; 88];
    VM.with(|cell| {
        let mut slot = cell.borrow_mut();
        let vm = slot.as_mut().unwrap();
        while warmup.elapsed().as_secs() < 3 {
            vm.calculate_hash(&input).unwrap();
            input[0] = input[0].wrapping_add(1);
        }
    });

    println!("Benchmarking 30 s...");
    let start = Instant::now();
    let mut count = 0u64;
    VM.with(|cell| {
        let mut slot = cell.borrow_mut();
        let vm = slot.as_mut().unwrap();
        while start.elapsed().as_secs() < 30 {
            vm.calculate_hash(&input).unwrap();
            input[0] = input[0].wrapping_add(1);
            count += 1;
        }
    });

    let elapsed = start.elapsed().as_secs_f64();
    let hps = count as f64 / elapsed;
    println!(
        "Result: {} hashes in {:.1}s = {:.2} H/s ({:.3} kH/s)",
        count,
        elapsed,
        hps,
        hps / 1000.0
    );
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("rxbench only runs on Linux (RandomX requires Linux). SSH into seed4 and run there.");
    std::process::exit(1);
}
