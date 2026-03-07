use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ferrous_node::network::batch::MessageBatcher;
use ferrous_node::network::message::REGTEST_MAGIC;
use ferrous_node::network::protocol::{InvVector, INV_BLOCK};

fn bench_message_batching(c: &mut Criterion) {
    c.bench_function("batch_1000_inv_items", |b| {
        b.iter(|| {
            let mut batcher = MessageBatcher::new(REGTEST_MAGIC);
            for i in 0..1000 {
                let mut hash = [0u8; 32];
                // Fill hash to prevent optimization
                hash[0] = (i % 255) as u8;

                let item = InvVector {
                    inv_type: INV_BLOCK,
                    hash,
                };
                batcher.add_inv(black_box(1), item);
            }
            batcher.flush(1);
        });
    });
}

fn bench_broadcast_cache(c: &mut Criterion) {
    c.bench_function("broadcast_cache_1000_checks", |b| {
        use ferrous_node::network::batch::BroadcastCache;

        b.iter(|| {
            let mut cache = BroadcastCache::new(1000);
            for i in 0..1000 {
                let mut hash = [0u8; 32];
                hash[0] = (i % 255) as u8;

                cache.already_sent(black_box(1), &hash);
                cache.mark_sent(1, hash);
            }
        });
    });
}

criterion_group!(benches, bench_message_batching, bench_broadcast_cache);
criterion_main!(benches);
