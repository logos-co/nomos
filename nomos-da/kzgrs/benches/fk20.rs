use std::{hint::black_box, sync::LazyLock};

use ark_bls12_381::{Bls12_381, Fr};
use ark_ff::BigInteger;
use ark_poly::{univariate::DensePolynomial, EvaluationDomain as _, GeneralEvaluationDomain};
use ark_poly_commit::kzg10::KZG10;
use divan::{counter::ItemsCount, Bencher};
use kzgrs::{
    bytes_to_polynomial,
    fk20::{fk20_batch_generate_elements_proofs, Toeplitz1Cache},
    GlobalParameters, BYTES_PER_FIELD_ELEMENT,
};
use ekzg_bls12_381::{
    fixed_base_msm::UsePrecomp, g1_batch_normalize, g2_batch_normalize, traits::*, G1Projective,
    G2Projective, Scalar,
};
use ekzg_multi_open::{
    commit_key::CommitKey, verification_key::VerificationKey, Prover, ProverInput, Verifier,
};
use ekzg_polynomial::poly_coeff::PolyCoeff;
use num_bigint::BigUint;
use rand::SeedableRng as _;
#[cfg(feature = "parallel")]
use rayon::iter::{IntoParallelIterator as _, ParallelIterator as _};

fn main() {
    divan::main();
}

static GLOBAL_PARAMETERS: LazyLock<GlobalParameters> = LazyLock::new(|| {
    let mut rng = rand::rngs::StdRng::seed_from_u64(1987);
    KZG10::<Bls12_381, DensePolynomial<Fr>>::setup(33791, true, &mut rng).unwrap()
});


#[divan::bench(args = [1_024], sample_count = 10, sample_size = 1)]
fn compute_fk20_proofs_for_size(bencher: Bencher, datasize: usize) {
    bencher
        .with_inputs(|| {
            let buff: Vec<_> = (0..BYTES_PER_FIELD_ELEMENT * datasize)
                .map(|i| (i % 255) as u8)
                .rev()
                .collect();
            let domain = GeneralEvaluationDomain::new(datasize*2).unwrap();
            let (_, poly) = bytes_to_polynomial::<BYTES_PER_FIELD_ELEMENT>(&buff, domain).unwrap();
            poly
        })
        .input_counter(move |_| ItemsCount::new(datasize*2))
        .bench_refs(|poly| {
            black_box(fk20_batch_generate_elements_proofs(
                poly,
                &GLOBAL_PARAMETERS,
                None,
            ))
        });
}


#[divan::bench(args = [1,2,4,8,16,32], sample_count = 10, sample_size = 1)]
fn compute_coset_fk20_proofs_for_size(bencher: Bencher, samplesize: usize) {
    bencher
        .with_inputs(|| {
            let buff: Vec<_> = (0..BYTES_PER_FIELD_ELEMENT * samplesize * 1024)
                .map(|i| (i % 255) as u8)
                .rev()
                .collect();
            let domain = GeneralEvaluationDomain::new(samplesize*2048).unwrap();
            let (_, poly) = bytes_to_polynomial::<BYTES_PER_FIELD_ELEMENT>(&buff, domain).unwrap();
            let poly = PolyCoeff::from(poly.coeffs.iter().map(|&e| {
                let mut vec = [0u8; 32];
                let bytes = e.0.to_bytes_be();
                vec[..bytes.len()].copy_from_slice(&bytes);
                Scalar::from_bytes_be(&vec).unwrap()
            }).collect::<Vec<Scalar>>());
            let (ck, _) = create_insecure_commit_verification_keys(samplesize * 1024usize, samplesize);

            let prover = Prover::new(
                ck,
                samplesize * 1024,
                samplesize,
                2048,
                UsePrecomp::Yes { width: 8 },
            );
            let num_proofs = prover.num_proofs();
            (prover, poly)
        })
        .input_counter(move |_| ItemsCount::new(samplesize*1024))
        .bench_refs(|(prover, poly)| {
            black_box( prover.compute_multi_opening_proofs(ProverInput::PolyCoeff(poly.clone())))
        });
}

#[cfg(feature = "parallel")]
#[divan::bench(args = [16, 32, 64, 128, 256, 512, 1_024, 2_048, 4_096], sample_count = 10, sample_size = 10)]
fn compute_parallel_fk20_proofs_for_size(bencher: Bencher, size: usize) {
    let thread_count: usize = rayon::max_num_threads().min(rayon::current_num_threads());
    bencher
        .with_inputs(|| {
            let buff: Vec<_> = (0..BYTES_PER_FIELD_ELEMENT * size)
                .map(|i| (i % 255) as u8)
                .rev()
                .collect();
            let domain = GeneralEvaluationDomain::new(size).unwrap();
            let (_, poly) = bytes_to_polynomial::<BYTES_PER_FIELD_ELEMENT>(&buff, domain).unwrap();
            poly
        })
        .input_counter(move |_| ItemsCount::new(size * thread_count))
        .bench_refs(|poly| {
            (0..thread_count).into_par_iter().for_each(|_| {
                let _ = fk20_batch_generate_elements_proofs(poly, &GLOBAL_PARAMETERS, None);
            });
            black_box(());
        });
}

#[divan::bench(args = [1_024], sample_count = 10, sample_size = 1)]
fn compute_fk20_proofs_for_size_with_cache(bencher: Bencher, size: usize) {
    bencher
        .with_inputs(|| {
            let buff: Vec<_> = (0..BYTES_PER_FIELD_ELEMENT * size)
                .map(|i| (i % 255) as u8)
                .rev()
                .collect();
            let domain = GeneralEvaluationDomain::new(size*2).unwrap();
            let (_, poly) = bytes_to_polynomial::<BYTES_PER_FIELD_ELEMENT>(&buff, domain).unwrap();
            let cache = Toeplitz1Cache::with_size(&GLOBAL_PARAMETERS, size*2);
            (poly, cache)
        })
        .input_counter(move |_| ItemsCount::new(size*2))
        .bench_refs(|(poly, cache)| {
            black_box(fk20_batch_generate_elements_proofs(
                poly,
                &GLOBAL_PARAMETERS,
                Some(cache),
            ))
        });
}

#[cfg(feature = "parallel")]
#[divan::bench(args = [16, 32, 64, 128, 256, 512, 1_024, 2_048, 4_096], sample_count = 10, sample_size = 10)]
fn compute_parallel_fk20_proofs_for_size_with_cache(bencher: Bencher, size: usize) {
    let thread_count: usize = rayon::max_num_threads().min(rayon::current_num_threads());
    bencher
        .with_inputs(|| {
            let buff: Vec<_> = (0..BYTES_PER_FIELD_ELEMENT * size)
                .map(|i| (i % 255) as u8)
                .rev()
                .collect();
            let domain = GeneralEvaluationDomain::new(size).unwrap();
            let (_, poly) = bytes_to_polynomial::<BYTES_PER_FIELD_ELEMENT>(&buff, domain).unwrap();
            let cache = Toeplitz1Cache::with_size(&GLOBAL_PARAMETERS, size);
            (poly, cache)
        })
        .input_counter(move |_| ItemsCount::new(size * thread_count))
        .bench_refs(|(poly, cache)| {
            (0..thread_count).into_par_iter().for_each(|_| {
                let _ = fk20_batch_generate_elements_proofs(poly, &GLOBAL_PARAMETERS, Some(cache));
            });
            black_box(());
        });
}

// We duplicate this to ensure that the version in the src code is only ever compiled with the test feature.
//
// This code should never be used outside of benchmarks and tests.
pub fn create_insecure_commit_verification_keys(
    size: usize,
    number_of_points_per_proof: usize
) -> (CommitKey, VerificationKey) {
    // A single proof will attest to the opening of 64 points.
    let multi_opening_size = number_of_points_per_proof;

    // We are making claims about a polynomial which has 4096 coefficients;
    let num_coefficients_in_polynomial = size;

    let g1_gen = G1Projective::generator();

    let secret = Scalar::random(&mut rand::thread_rng());

    let mut g1_points = Vec::new();
    let mut current_secret_pow = secret;
    for _ in 0..num_coefficients_in_polynomial {
        g1_points.push(g1_gen * current_secret_pow);
        current_secret_pow *= secret;
    }
    let g1_points = g1_batch_normalize(&g1_points);

    let ck = CommitKey::new(g1_points.clone());

    let mut g2_points = Vec::new();
    let mut current_secret_pow = secret;
    let g2_gen = G2Projective::generator();
    // The setup needs 65 g1 elements for the verification key, in order
    // to commit to the remainder polynomial.
    for _ in 0..=multi_opening_size {
        g2_points.push(g2_gen * current_secret_pow);
        current_secret_pow *= secret;
    }
    let g2_points = g2_batch_normalize(&g2_points);

    let vk = VerificationKey::new(
        g1_points[0..=multi_opening_size].to_vec(),
        g2_points,
        multi_opening_size,
        num_coefficients_in_polynomial,
    );

    (ck, vk)
}