use ark_bls12_381::{Fr, G1Affine, G1Projective};
use ark_ec::AffineRepr;
use ark_ff::BigInt;
use ark_poly::{EvaluationDomain, GeneralEvaluationDomain};
use divan::counter::ItemsCount;
use divan::{black_box, Bencher};
fn main() {
    divan::main()
}

#[divan::bench(args = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096])]
fn compute_ark_fft_for_size(bencher: Bencher, size: usize) {
    bencher
        .with_inputs(|| {
            let domain = GeneralEvaluationDomain::<Fr>::new(size).unwrap();
            let buff: Vec<G1Projective> = (0..size)
                .map(|i| G1Affine::identity().mul_bigint(BigInt::<4>::from(i as u64)))
                .collect();
            (buff, domain)
        })
        .input_counter(move |_| ItemsCount::new(size))
        .bench_refs(|(buff, domain)| black_box(domain.fft(buff)));
}
