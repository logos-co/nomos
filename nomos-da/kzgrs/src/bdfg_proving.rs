use super::{Commitment, Evaluations, GlobalParameters, PolynomialEvaluationDomain, Proof};
use crate::fk20::{fk20_batch_generate_elements_proofs, Toeplitz1Cache};
use ark_bls12_381::Fr;
use ark_ff::{Field, PrimeField};
use ark_poly::EvaluationDomain;
use ark_serialize::CanonicalSerialize;
use blake2::{
    digest::{Digest as _, Update, VariableOutput},
    Blake2bVar,
};
use std::io::Cursor;

/// Module with implementation of [Efficient polynomial commitment schemes for multiple points and polynomials](https://eprint.iacr.org/2020/081.pdf)
pub fn generate_row_commitments_hash(commitments: &[Commitment]) -> Vec<u8> {
    let mut hasher = Blake2bVar::new(32).expect("Hasher should be able to build");
    commitments.iter().for_each(|c| {
        let mut buffer = Cursor::new(Vec::new());
        c.serialize_uncompressed(&mut buffer)
            .expect("serialization");
        hasher.update(&buffer.into_inner());
    });
    let mut buffer = [0; 32];
    hasher
        .finalize_variable(&mut buffer)
        .expect("Hashing should succeed");
    buffer.to_vec()
}

pub fn compute_aggregated_polynomial(
    polynomials: &[Evaluations],
    aggregated_commitments_hash: &[u8],
    domain: PolynomialEvaluationDomain,
) -> Evaluations {
    let h = Fr::from_le_bytes_mod_order(aggregated_commitments_hash);
    let evals: Vec<Fr> = (0..domain.size())
        .map(|i| {
            polynomials
                .iter()
                .map(|poly| poly.evals[i] * h.pow([i as u64 - 1]))
                .sum()
        })
        .collect();
    Evaluations::from_vec_and_domain(evals, domain)
}

pub fn generate_aggregated_proof(
    polynomials: &[Evaluations],
    commitments: &[Commitment],
    domain: PolynomialEvaluationDomain,
    global_parameters: &GlobalParameters,
    toeplitz1cache: Option<&Toeplitz1Cache>,
) -> Vec<Proof> {
    let rows_commitments_hash = generate_row_commitments_hash(commitments);
    let aggregated_commitments_evals =
        compute_aggregated_polynomial(polynomials, &rows_commitments_hash, domain);
    let aggregated_commitments_polynomial = aggregated_commitments_evals.interpolate_by_ref();
    fk20_batch_generate_elements_proofs(
        &aggregated_commitments_polynomial,
        global_parameters,
        toeplitz1cache,
    )
}
