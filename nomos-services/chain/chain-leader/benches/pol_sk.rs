use chain_leader::pol::pol_sk_generator;
use cryptarchia_engine::Slot;
use divan::{Bencher, black_box, counter::ItemsCount};
use groth16::{Fr, fr_from_bytes};

fn main() {
    divan::main();
}

#[divan::bench(sample_count = 1, sample_size = 1)]
fn compute_pol_slot_secret(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let seed: Fr = fr_from_bytes(b"1987").unwrap();
            let slot: Slot = Slot::new(1987);
            (slot, seed)
        })
        .bench_values(|(slot, seed)| black_box(pol_sk_generator(slot, seed)));
}
