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

#[divan::bench(sample_count = 10, sample_size = 10)]
fn precompute_slot_secret(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let seed: Fr = fr_from_bytes(b"1987").unwrap();
            seed
        })
        .bench_values(|seed| black_box(MerklePolCache::new(seed, Slot::new(0), 25, 20)));
}

#[divan::bench(sample_count = 10, sample_size = 10)]
fn compute_non_cached_subtree(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let seed: Fr = fr_from_bytes(b"1987").unwrap();
            seed
        })
        .bench_values(|seed| black_box(MerklePolSubtree::new(seed, 5).merkle_path_for_index(0)));
}

#[divan::bench(sample_count = 10, sample_size = 10)]
fn precompute_leaves(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let seed: Fr = fr_from_bytes(b"1987").unwrap();
            seed
        })
        .bench_values(|seed| {
            black_box(
                MerklePolCache::leaves_from_seed(seed)
                    .take(2usize.pow(20))
                    .collect::<Vec<_>>(),
            )
        });
}
