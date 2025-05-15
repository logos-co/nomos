use std::{
    borrow::Cow,
    ops::{Mul as _, Neg as _},
};

use ark_bls12_381::{Bls12_381, Fr};
use ark_ec::pairing::Pairing as _;
use ark_poly::{
    univariate::DensePolynomial, DenseUVPolynomial as _, EvaluationDomain as _,
    GeneralEvaluationDomain,
};
use ark_poly_commit::kzg10::{Commitment, Powers, Proof, UniversalParams, KZG10};
use num_traits::{One as _, Zero as _};

use crate::{common::KzgRsError, Evaluations};

/// Commit to a polynomial where each of the evaluations are over `w(i)` for the
/// degree of the polynomial being omega (`w`) the root of unity (2^x).
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
    evaluations: &Evaluations,
    global_parameters: &UniversalParams<Bls12_381>,
    domain: GeneralEvaluationDomain<Fr>,
) -> Result<Proof<Bls12_381>, KzgRsError> {
    let u = domain.element(element_index);
    if u.is_zero() {
        return Err(KzgRsError::DivisionByZeroPolynomial);
    }

    // Instead of evaluating over the polynomial, we can reuse the evaluation points
    // from the rs encoding let v = polynomial.evaluate(&u);
    let v = evaluations.evals[element_index];
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
#[must_use]
pub fn verify_element_proof(
    element_index: usize,
    element: &Fr,
    commitment: &Commitment<Bls12_381>,
    proof: &Proof<Bls12_381>,
    domain: GeneralEvaluationDomain<Fr>,
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
    use std::sync::LazyLock;

    use ark_bls12_381::{Bls12_381, Fr};
    use ark_poly::{
        univariate::DensePolynomial, DenseUVPolynomial as _, EvaluationDomain as _,
        GeneralEvaluationDomain,
    };
    use ark_poly_commit::kzg10::{UniversalParams, KZG10};
    use rand::{thread_rng, Fill as _};
    use rayon::{
        iter::{IndexedParallelIterator as _, ParallelIterator as _},
        prelude::IntoParallelRefIterator as _,
    };

    use crate::{
        common::bytes_to_polynomial,
        kzg::{commit_polynomial, generate_element_proof, verify_element_proof},
    };

    const COEFFICIENTS_SIZE: usize = 16;
    static GLOBAL_PARAMETERS: LazyLock<UniversalParams<Bls12_381>> = LazyLock::new(|| {
        let mut rng = rand::thread_rng();
        KZG10::<Bls12_381, DensePolynomial<Fr>>::setup(
            crate::kzg::test::COEFFICIENTS_SIZE - 1,
            true,
            &mut rng,
        )
        .unwrap()
    });

    static DOMAIN: LazyLock<GeneralEvaluationDomain<Fr>> =
        LazyLock::new(|| GeneralEvaluationDomain::new(COEFFICIENTS_SIZE).unwrap());
    #[test]
    fn test_poly_commit() {
        let poly = DensePolynomial::from_coefficients_vec((0..10).map(Fr::from).collect());
        assert!(commit_polynomial(&poly, &GLOBAL_PARAMETERS).is_ok());
    }

    #[test]
    fn generate_proof_and_validate() {
        let mut bytes: [u8; 310] = [0; 310];
        let mut rng = thread_rng();
        bytes.try_fill(&mut rng).unwrap();
        let (eval, poly) = bytes_to_polynomial::<31>(&bytes, *DOMAIN).unwrap();
        let commitment = commit_polynomial(&poly, &GLOBAL_PARAMETERS).unwrap();
        let proofs: Vec<_> = (0..10)
            .map(|i| generate_element_proof(i, &poly, &eval, &GLOBAL_PARAMETERS, *DOMAIN).unwrap())
            .collect();

        eval.evals
            .par_iter()
            .zip(proofs.par_iter())
            .enumerate()
            .for_each(|(i, (element, proof))| {
                for ii in i..10 {
                    if ii == i {
                        // verifying works
                        assert!(verify_element_proof(
                            ii,
                            element,
                            &commitment,
                            proof,
                            *DOMAIN,
                            &GLOBAL_PARAMETERS
                        ));
                    } else {
                        // Verification should fail for other points
                        assert!(!verify_element_proof(
                            ii,
                            element,
                            &commitment,
                            proof,
                            *DOMAIN,
                            &GLOBAL_PARAMETERS
                        ));
                    }
                }
            });
    }
}
