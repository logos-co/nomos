use chain_leader::pol::{
    SlotSecret,
    merkle::{MerklePolCache, MerklePolSubtree},
};
use cryptarchia_engine::Slot;
use divan::{Bencher, black_box, counter::ItemsCount};
use groth16::{Fr, fr_from_bytes};

fn main() {
    divan::main();
}

#[divan::bench(sample_count = 1, sample_size = 1)]
fn precompute_slot_secret(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let seed: Fr = fr_from_bytes(b"1987").unwrap();
            seed
        })
        .bench_values(|seed| black_box(MerklePolCache::new(seed, 25, 20)));
}

#[divan::bench(sample_count = 1, sample_size = 1)]
fn precompute_leaves(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let seed: Fr = fr_from_bytes(b"1987").unwrap();
            seed
        })
        .bench_values(|seed| {
            black_box(
                MerklePolCache::leaves_from_seed(seed)
                    .take(2usize.pow(25))
                    .collect::<Vec<_>>(),
            )
        });
}
