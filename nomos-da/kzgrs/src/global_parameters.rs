use std::error::Error;

use ark_bls12_381::{Bls12_381, fr::Fr};
use ark_poly::polynomial::univariate::DensePolynomial;
use ark_poly_commit::kzg10::{KZG10, UniversalParams};
use ark_serialize::CanonicalDeserialize as _;
use rand::Rng;

use super::GlobalParameters;

pub fn global_parameters_from_randomness<R: Rng>(rng: &mut R) -> GlobalParameters {
    KZG10::<Bls12_381, DensePolynomial<Fr>>::setup(8192, true, rng).unwrap()
}

pub fn global_parameters_from_file(file_path: &str) -> Result<GlobalParameters, Box<dyn Error>> {
    let serialized_data = std::fs::read(file_path)?;

    let params =
        UniversalParams::<Bls12_381>::deserialize_uncompressed_unchecked(&*serialized_data)?;
    Ok(params)
}
