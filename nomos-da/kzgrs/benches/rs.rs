use ark_bls12_381::Fr;
use ark_poly::{EvaluationDomain as _, GeneralEvaluationDomain};
use divan::{black_box, counter::BytesCount, Bencher};
use kzgrs::{
    common::bytes_to_polynomial,
    rs::{decode, encode},
};
use rand::{thread_rng, RngCore as _};

fn main() {
    divan::main();
}

#[divan::bench(args = [3_224])]
fn rs_encode(bencher: Bencher, size: usize) {
    bencher
        .with_inputs(move || {
            let mut buffer = vec![0u8; size];
            thread_rng().fill_bytes(&mut buffer);
            buffer
        })
        .input_counter(move |buff| BytesCount::new(buff.len()))
        .bench_refs(|buff| {
            let domain = GeneralEvaluationDomain::<Fr>::new(size).unwrap();
            let (_, poly) = bytes_to_polynomial::<31>(buff, domain).unwrap();
            let domain = GeneralEvaluationDomain::<Fr>::new(size * 2).unwrap();
            black_box(move || encode(&poly, domain))
        });
}

#[divan::bench(args = [16_399, 32_798, 65_565, 131_099, 262_167, 524_241, 1_048_606], sample_size = 10, sample_count = 100)]
fn rs_decode(bencher: Bencher, size: usize) {
    bencher
        .with_inputs(move || {
            let mut buffer = vec![0u8; size];
            thread_rng().fill_bytes(&mut buffer);
            let domain = GeneralEvaluationDomain::<Fr>::new(size).unwrap();
            let (_, poly) = bytes_to_polynomial::<31>(&buffer, domain).unwrap();
            let domain = GeneralEvaluationDomain::<Fr>::new(size * 2).unwrap();
            encode(&poly, domain)
        })
        .input_counter(move |_buff| BytesCount::new(size * 31))
        .bench_values(|buff| {
            black_box(move || {
                let domain = GeneralEvaluationDomain::<Fr>::new(size).unwrap();
                let missing_data: Vec<_> = std::iter::repeat(None)
                    .take(size)
                    .chain(buff.evals[size..].iter().copied().map(Some))
                    .collect();
                decode(size, &missing_data, domain)
            })
        });
}
