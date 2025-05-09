use std::sync::LazyLock;

use ark_bls12_381::{Bls12_381, Fr};
use ark_poly::{univariate::DensePolynomial, EvaluationDomain as _, GeneralEvaluationDomain};
use ark_poly_commit::kzg10::{UniversalParams, KZG10};
use divan::{black_box, counter::ItemsCount, Bencher};
use kzgrs::{
    common::bytes_to_polynomial_unchecked,
    kzg::{commit_polynomial, generate_element_proof, verify_element_proof},
};
use rand::RngCore as _;
#[cfg(feature = "parallel")]
use rayon::iter::IntoParallelIterator as _;
#[cfg(feature = "parallel")]
use rayon::iter::ParallelIterator as _;

fn main() {
    divan::main();
}

// This allocator setting seems like it doesn't work on windows. Disable for
// now, but letting it here in case it's needed at some specific point.
// #[global_allocator]
// static ALLOC: AllocProfiler = AllocProfiler::system();

static GLOBAL_PARAMETERS: LazyLock<UniversalParams<Bls12_381>> = LazyLock::new(|| {
    let mut rng = rand::thread_rng();
    KZG10::<Bls12_381, DensePolynomial<Fr>>::setup(4096, true, &mut rng).unwrap()
});

fn rand_data_elements(elements_count: usize, chunk_size: usize) -> Vec<u8> {
    let mut buff = vec![0u8; elements_count * chunk_size];
    rand::thread_rng().fill_bytes(&mut buff);
    buff
}

const CHUNK_SIZE: usize = 31;

#[divan::bench(args = [16, 32, 64, 128, 256, 512, 1_024, 2_048, 4_096])]
fn commit_single_polynomial_with_element_count(bencher: Bencher, element_count: usize) {
    bencher
        .with_inputs(|| {
            let domain = GeneralEvaluationDomain::new(element_count).unwrap();
            let data = rand_data_elements(element_count, CHUNK_SIZE);
            bytes_to_polynomial_unchecked::<CHUNK_SIZE>(&data, domain)
        })
        .input_counter(move |(_evals, _poly)| ItemsCount::new(1usize))
        .bench_refs(|(_evals, poly)| black_box(commit_polynomial(poly, &GLOBAL_PARAMETERS)));
}

#[cfg(feature = "parallel")]
#[divan::bench(args = [16, 32, 64, 128, 256, 512, 1_024, 2_048, 4_096])]
fn commit_polynomial_with_element_count_parallelized(bencher: Bencher, element_count: usize) {
    let threads = 8usize;
    bencher
        .with_inputs(|| {
            let domain = GeneralEvaluationDomain::new(element_count).unwrap();
            let data = rand_data_elements(element_count, CHUNK_SIZE);
            bytes_to_polynomial_unchecked::<CHUNK_SIZE>(&data, domain)
        })
        .input_counter(move |(_evals, _poly)| ItemsCount::new(threads))
        .bench_refs(|(_evals, poly)| {
            let _commitments: Vec<_> = (0..threads)
                .into_par_iter()
                .map(|_| commit_polynomial(poly, &GLOBAL_PARAMETERS))
                .collect();
        });
}

#[divan::bench(args = [128, 256, 512, 1_024, 2_048, 4_096])]
fn compute_single_proof(bencher: Bencher, element_count: usize) {
    bencher
        .with_inputs(|| {
            let domain = GeneralEvaluationDomain::new(element_count).unwrap();
            let data = rand_data_elements(element_count, CHUNK_SIZE);
            (
                bytes_to_polynomial_unchecked::<CHUNK_SIZE>(&data, domain),
                domain,
            )
        })
        .input_counter(|_| ItemsCount::new(1usize))
        .bench_refs(|((evals, poly), domain)| {
            black_box(generate_element_proof(
                7,
                poly,
                evals,
                &GLOBAL_PARAMETERS,
                *domain,
            ))
        });
}

#[divan::bench(args = [128, 256, 512, 1_024], sample_count = 3, sample_size = 5)]
fn compute_batch_proofs(bencher: Bencher, element_count: usize) {
    bencher
        .with_inputs(|| {
            let domain = GeneralEvaluationDomain::new(element_count).unwrap();
            let data = rand_data_elements(element_count, CHUNK_SIZE);
            (
                bytes_to_polynomial_unchecked::<CHUNK_SIZE>(&data, domain),
                domain,
            )
        })
        .input_counter(move |_| ItemsCount::new(element_count))
        .bench_refs(|((evals, poly), domain)| {
            for i in 0..element_count {
                black_box(
                    generate_element_proof(i, poly, evals, &GLOBAL_PARAMETERS, *domain).unwrap(),
                );
            }
        });
}

// This is a test on how will perform by having a wrapping rayon on top of the
// proof computation ark libraries already use rayon underneath so no great
// improvements are probably come up from this. But it should help reusing the
// same thread pool for all jobs saving a little time.
#[cfg(feature = "parallel")]
#[divan::bench(args = [128, 256, 512, 1_024], sample_count = 3, sample_size = 5)]
fn compute_parallelize_batch_proofs(bencher: Bencher, element_count: usize) {
    bencher
        .with_inputs(|| {
            let domain = GeneralEvaluationDomain::new(element_count).unwrap();
            let data = rand_data_elements(element_count, CHUNK_SIZE);
            (
                bytes_to_polynomial_unchecked::<CHUNK_SIZE>(&data, domain),
                domain,
            )
        })
        .input_counter(move |_| ItemsCount::new(element_count))
        .bench_refs(|((evals, poly), domain)| {
            (0..element_count).into_par_iter().for_each(|i| {
                generate_element_proof(i, poly, evals, &GLOBAL_PARAMETERS, *domain).unwrap();
            });
            black_box(());
        });
}

#[divan::bench]
fn verify_single_proof(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let element_count = 10;
            let domain = GeneralEvaluationDomain::new(element_count).unwrap();
            let data = rand_data_elements(element_count, CHUNK_SIZE);
            let (eval, poly) = bytes_to_polynomial_unchecked::<CHUNK_SIZE>(&data, domain);
            let commitment = commit_polynomial(&poly, &GLOBAL_PARAMETERS).unwrap();
            let proof =
                generate_element_proof(0, &poly, &eval, &GLOBAL_PARAMETERS, domain).unwrap();
            (0usize, eval.evals[0], commitment, proof, domain)
        })
        .input_counter(|_| ItemsCount::new(1usize))
        .bench_refs(|(index, elemnent, commitment, proof, domain)| {
            black_box(verify_element_proof(
                *index,
                elemnent,
                commitment,
                proof,
                *domain,
                &GLOBAL_PARAMETERS,
            ))
        });
}
