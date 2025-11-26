use chain_leader::pol::{SlotSecret, merkle::MerklePol, pol_sk_generator};
use cryptarchia_engine::Slot;
use divan::{Bencher, black_box, counter::ItemsCount};
use groth16::{Fr, fr_from_bytes};

fn main() {
    divan::main();
}

// #[divan::bench(sample_count = 1, sample_size = 1)]
// fn compute_pol_slot_secret(bencher: Bencher) {
//     bencher
//         .with_inputs(|| {
//             let seed: Fr = fr_from_bytes(b"1987").unwrap();
//             let slot: Slot = Slot::new(1987);
//             (slot, seed)
//         })
//         .bench_values(|(slot, seed)| black_box(pol_sk_generator(slot,
// seed))); }

// #[divan::bench(sample_count = 1, sample_size = 10)]
// fn advance_mmr(bencher: Bencher) {
//     bencher
//         .with_inputs(|| {
//             let seed: SlotSecret = fr_from_bytes(b"1987").unwrap().into();
//             let merkle_pol = MerklePol::new(seed);
//             merkle_pol
//         })
//         .bench_local_refs(|merkle_pol|
// black_box(MerklePol::next(merkle_pol))); }

#[divan::bench(sample_count = 1, sample_size = 1)]
fn precompute_mmr(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let seed: Fr = fr_from_bytes(b"1987").unwrap();
            seed
        })
        .bench_values(|seed| black_box(MerklePol::new(seed)));
}

// #[divan::bench(sample_count = 1, sample_size = 1)]
// fn reduce(bencher: Bencher) {
//     bencher
//         .with_inputs(|| {
//             let seed: Fr = fr_from_bytes(b"1987").unwrap();
//             MerklePol::new(seed, 10)
//         })
//         .bench_values(|merkle_pol| black_box(merkle_pol.reduce()));
// }
