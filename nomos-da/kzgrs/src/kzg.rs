use crate::common::KzgRsError;
use ark_bls12_381::{Bls12_381, Fr};
use ark_ec::pairing::Pairing;
use ark_poly::univariate::DensePolynomial;
use ark_poly::{DenseUVPolynomial, EvaluationDomain, GeneralEvaluationDomain, Polynomial};
use ark_poly_commit::kzg10::{Commitment, Powers, Proof, UniversalParams, KZG10};
use num_traits::One;
use std::borrow::Cow;
use std::ops::{Mul, Neg};

/// Commit to a polynomial where each of the evaluations are over `w(i)` for the degree
/// of the polynomial being omega (`w`) the root of unity (2^x).
pub fn commit_polynomial(
    polynomial: &DensePolynomial<Fr>,
    global_parameters: &UniversalParams<Bls12_381>,
) -> Result<Commitment<Bls12_381>, KzgRsError> {
    let roots_of_unity = Powers {
        powers_of_g: Cow::Borrowed(&global_parameters.powers_of_g),
        powers_of_gamma_g: Cow::Owned(vec![]),
    };
    KZG10::commit(&roots_of_unity, polynomial, None, None)
        .map_err(KzgRsError::PolyCommitError)
        .map(|(commitment, _)| commitment)
}

/// Compute a witness polynomial in that satisfies `witness(x) = (f(x)-v)/(x-u)`
pub fn generate_element_proof(
    element_index: usize,
    polynomial: &DensePolynomial<Fr>,
    global_parameters: &UniversalParams<Bls12_381>,
    domain: &GeneralEvaluationDomain<Fr>,
) -> Result<Proof<Bls12_381>, KzgRsError> {
    let u = domain.element(element_index);
    let v = polynomial.evaluate(&u);
    let f_x_v = polynomial + &DensePolynomial::<Fr>::from_coefficients_vec(vec![-v]);
    let x_u = DensePolynomial::<Fr>::from_coefficients_vec(vec![-u, Fr::one()]);
    let witness_polynomial: DensePolynomial<_> = &f_x_v / &x_u;
    let proof = commit_polynomial(&witness_polynomial, global_parameters)?;
    let proof = Proof {
        w: proof.0,
        random_v: None,
    };
    Ok(proof)
}

/// Verify proof for a single element
pub fn verify_element_proof(
    element_index: usize,
    element: &Fr,
    commitment: &Commitment<Bls12_381>,
    proof: &Proof<Bls12_381>,
    domain: &GeneralEvaluationDomain<Fr>,
    global_parameters: &UniversalParams<Bls12_381>,
) -> bool {
    let u = domain.element(element_index);
    let v = element;
    let commitment_check_g1 = commitment.0 + global_parameters.powers_of_g[0].mul(v).neg();
    let proof_check_g2 = global_parameters.beta_h + global_parameters.h.mul(u).neg();
    let lhs = Bls12_381::pairing(commitment_check_g1, global_parameters.h);
    let rhs = Bls12_381::pairing(proof.w, proof_check_g2);
    lhs == rhs
}

#[cfg(test)]
mod test {
    use crate::common::{bytes_to_evaluations, bytes_to_polynomial};
    use crate::kzg::{commit_polynomial, generate_element_proof, verify_element_proof};
    use ark_bls12_381::{Bls12_381, Fr};
    use ark_poly::univariate::DensePolynomial;
    use ark_poly::{DenseUVPolynomial, EvaluationDomain, GeneralEvaluationDomain};
    use ark_poly_commit::kzg10::{UniversalParams, KZG10};
    use once_cell::sync::Lazy;
    use rand::{thread_rng, Fill};

    const COEFFICIENTS_SIZE: usize = 16;
    static GLOBAL_PARAMETERS: Lazy<UniversalParams<Bls12_381>> = Lazy::new(|| {
        let mut rng = rand::thread_rng();
        KZG10::<Bls12_381, DensePolynomial<Fr>>::setup(
            crate::kzg::test::COEFFICIENTS_SIZE - 1,
            true,
            &mut rng,
        )
        .unwrap()
    });

    static DOMAIN: Lazy<GeneralEvaluationDomain<Fr>> =
        Lazy::new(|| GeneralEvaluationDomain::new(COEFFICIENTS_SIZE).unwrap());
    #[test]
    fn test_poly_commit() {
        let poly = DensePolynomial::from_coefficients_vec((0..10).map(|i| Fr::from(i)).collect());
        assert!(matches!(
            commit_polynomial(&poly, &GLOBAL_PARAMETERS),
            Ok(_)
        ));
    }

    #[test]
    fn generate_proof_and_validate() {
        let mut bytes: [u8; 310] = [0; 310];
        let mut rng = thread_rng();
        bytes.try_fill(&mut rng).unwrap();
        let evaluations = bytes_to_evaluations::<31>(&bytes, *DOMAIN).evals;
        let (_, poly) = bytes_to_polynomial::<31>(&bytes, *DOMAIN).unwrap();
        let commitment = commit_polynomial(&poly, &GLOBAL_PARAMETERS).unwrap();
        let proofs: Vec<_> = (0..10)
            .map(|i| generate_element_proof(i, &poly, &GLOBAL_PARAMETERS, &DOMAIN).unwrap())
            .collect();
        for (i, (element, proof)) in evaluations.iter().zip(proofs.iter()).enumerate() {
            // verifying works
            assert!(verify_element_proof(
                i,
                element,
                &commitment,
                proof,
                &DOMAIN,
                &GLOBAL_PARAMETERS
            ));
            // verification fails for other items
            for ii in i + 1..10 {
                assert!(!verify_element_proof(
                    ii,
                    element,
                    &commitment,
                    proof,
                    &DOMAIN,
                    &GLOBAL_PARAMETERS
                ));
            }
        }
    }
}
