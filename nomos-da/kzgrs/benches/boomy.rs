use std::sync::LazyLock;

use ark_bls12_381::{Bls12_381, Fr};
use ark_poly::{EvaluationDomain as _, GeneralEvaluationDomain, Polynomial};
use ark_poly_commit::kzg10::{UniversalParams, KZG10};
use divan::{black_box, counter::ItemsCount, Bencher};
use kzgrs::{
    common::bytes_to_polynomial_unchecked,
    boomy::BivariateUniversalParams
};
use rand::{random, RngCore as _};
use rand::seq::index::sample;
#[cfg(feature = "parallel")]
use rayon::iter::IntoParallelIterator as _;
#[cfg(feature = "parallel")]
use rayon::iter::ParallelIterator as _;
use kzgrs::boomy::{bivariate_setup, generate_bivariate_proof};

fn main() {
    divan::main();
}

// This allocator setting seems like it doesn't work on windows. Disable for
// now, but letting it here in case it's needed at some specific point.
// #[global_allocator]
// static ALLOC: AllocProfiler = AllocProfiler::system();

static GLOBAL_PARAMETERS: LazyLock<BivariateUniversalParams<Bls12_381>> =
    LazyLock::new(|| {
        let mut rng = rand::thread_rng();
        bivariate_setup(1023, 31, &mut rng).unwrap()
    });

const CHUNK_SIZE: usize = 31;

#[divan::bench(args = [128, 256, 512, 1_024, 2_048, 4_096])]
fn compute_bivariate_single_five_point_proof(bencher: Bencher, element_count: usize) {
    bencher
        .with_inputs(|| {
            let y_domain = GeneralEvaluationDomain::new(5).unwrap();
            let polynomial = kzgrs::bivariate::DensePolynomial::rand(element_count-1,4, &mut rand::thread_rng());
            let sample_index: usize = (random::<u64>() % element_count as u64) as usize;
            (sample_index, polynomial, y_domain)
        })
        .input_counter(|_| ItemsCount::new(1usize))
        .bench_refs(|(sample_index, polynomial, y_domain)| {
        black_box(generate_bivariate_proof(*sample_index,
                polynomial,
                *y_domain,
                &GLOBAL_PARAMETERS
            ))
        });
}

#[divan::bench(args = [128, 256, 512, 1_024], sample_count = 3, sample_size = 1)]
fn compute_bivariate_batch_thirty_two_points_proof(bencher: Bencher, element_count: usize) {
    bencher
        .with_inputs(|| {
            let y_domain = GeneralEvaluationDomain::new(32).unwrap();
            let polynomial = kzgrs::bivariate::DensePolynomial::rand((element_count/32)-1,31, &mut rand::thread_rng());
            (polynomial, y_domain)
        })
        .input_counter(move |_| ItemsCount::new(element_count))
        .bench_refs(|(polynomial, y_domain)| {
            for i in 0..(element_count/32) {
                black_box(generate_bivariate_proof(
                    i,
                    polynomial,
                    *y_domain,
                    &GLOBAL_PARAMETERS,
                ));
            }
        });
}

/*#[divan::bench(args = [128, 256, 512, 1_024], sample_count = 3, sample_size = 32)]
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

#[divan::bench(args = [128, 256, 512, 1_024, 2_048, 4_096])]
fn compute_single_five_points_proof(bencher: Bencher, element_count: usize) {
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
            black_box(generate_multiple_element_proof(
                &(0..5).collect::<Vec<usize>>(),
                poly,
                *domain,
                &MULTIPLE_GLOBAL_PARAMETERS,
            ));
        });
}

#[divan::bench(args = [128, 256, 512, 1_024], sample_count = 3, sample_size = 1)]
fn compute_batch_thirty_two_points_proof(bencher: Bencher, element_count: usize) {
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
            for i in 0..(element_count) {
                black_box(generate_multiple_element_proof(
                    &(i * 32..i * 32 + 32).collect::<Vec<usize>>(),
                    poly,
                    *domain,
                    &MULTIPLE_GLOBAL_PARAMETERS,
                ));
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
*/