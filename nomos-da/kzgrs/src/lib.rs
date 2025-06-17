pub mod common;
pub mod fk20;
pub mod global_parameters;
pub mod kzg;

pub mod bdfg_proving;
pub mod rs;

use ark_bls12_381::{Bls12_381, Fr};
use ark_poly::{univariate::DensePolynomial, GeneralEvaluationDomain};
use ark_poly_commit::{kzg10, sonic_pc::UniversalParams};
pub use common::{bytes_to_evaluations, bytes_to_polynomial, KzgRsError};
pub use global_parameters::{global_parameters_from_file, global_parameters_from_randomness};
pub use kzg::{commit_polynomial, generate_element_proof, verify_element_proof};
pub use rs::{decode, encode};

pub type Commitment = kzg10::Commitment<Bls12_381>;
pub type Proof = kzg10::Proof<Bls12_381>;
pub type FieldElement = Fr;
pub type Polynomial = DensePolynomial<Fr>;
pub type Evaluations = ark_poly::Evaluations<Fr>;
pub type PolynomialEvaluationDomain = GeneralEvaluationDomain<Fr>;

pub type GlobalParameters = UniversalParams<Bls12_381>;

pub const BYTES_PER_FIELD_ELEMENT: usize = size_of::<Fr>();
