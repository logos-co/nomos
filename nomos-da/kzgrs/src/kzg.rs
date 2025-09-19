use std::{
    borrow::Cow,
    collections::BTreeMap,
    ops::{Mul as _, Neg as _},
};

use ark_bls12_381::{Bls12_381, Fr, G2Projective};
use ark_ec::{pairing::Pairing, scalar_mul::fixed_base::FixedBase, CurveGroup};
use ark_ff::{Field, PrimeField, UniformRand};
use ark_poly::{
    univariate::{DenseOrSparsePolynomial, DensePolynomial},
    DenseUVPolynomial as _, EvaluationDomain as _, GeneralEvaluationDomain, Polynomial,
};
use ark_poly_commit::{
    kzg10::{Commitment, Powers, Proof, UniversalParams, KZG10},
    Error,
};
use ark_std::rand::RngCore;
use num_traits::{One as _, Zero as _};

use crate::{common::KzgRsError, Evaluations};

pub struct MultiplePointUniversalParams<E: Pairing> {
    /// Group elements of the form `{ \beta^i G }`, where `i` ranges from 0 to
    /// `degree`.
    pub powers_of_g: Vec<E::G1Affine>,
    /// Group elements of the form `{ \beta^i \gamma G }`, where `i` ranges from
    /// 0 to `degree`.
    pub powers_of_gamma_g: BTreeMap<usize, E::G1Affine>,
    /// The generator of G2.
    pub h: E::G2Affine,
    /// \beta times the above generator of G2.
    pub powers_of_h: Vec<E::G2Affine>,
    /// Group elements of the form `{ \beta^i G2 }`, where `i` ranges from `0`
    /// to `-degree`.
    pub neg_powers_of_h: BTreeMap<usize, E::G2Affine>,
    /// The generator of G2, prepared for use in pairings.
    pub prepared_h: E::G2Prepared,
    /// \beta times the above generator of G2, prepared for use in pairings.
    pub prepared_beta_h: E::G2Prepared,
}

pub fn multiple_point_setup<R: RngCore>(
    max_degree: usize,
    max_evals: usize,
    produce_g2_powers: bool,
    rng: &mut R,
) -> Result<MultiplePointUniversalParams<Bls12_381>, Error> {
    if max_degree < 1 {
        return Err(Error::DegreeIsZero);
    }
    let beta = Fr::rand(rng);
    let g = <Bls12_381 as Pairing>::G1::rand(rng);
    let gamma_g = <Bls12_381 as Pairing>::G1::rand(rng);
    let h = <Bls12_381 as Pairing>::G2::rand(rng);

    let mut powers_of_beta = vec![Fr::ONE];

    let mut cur = beta;
    for _ in 0..max_degree {
        powers_of_beta.push(cur);
        cur *= &beta;
    }

    let window_size = FixedBase::get_mul_window_size(max_degree + 1);

    let scalar_bits = Fr::MODULUS_BIT_SIZE as usize;
    let g_table = FixedBase::get_window_table(scalar_bits, window_size, g);
    let powers_of_g = FixedBase::msm::<<Bls12_381 as Pairing>::G1>(
        scalar_bits,
        window_size,
        &g_table,
        &powers_of_beta,
    );
    let gamma_g_table = FixedBase::get_window_table(scalar_bits, window_size, gamma_g);
    let mut powers_of_gamma_g = FixedBase::msm::<<Bls12_381 as Pairing>::G1>(
        scalar_bits,
        window_size,
        &gamma_g_table,
        &powers_of_beta,
    );
    // Add an additional power of gamma_g, because we want to be able to support
    // up to D queries.
    powers_of_gamma_g.push(powers_of_gamma_g.last().unwrap().mul(&beta));

    let powers_of_g = <Bls12_381 as Pairing>::G1::normalize_batch(&powers_of_g);
    let powers_of_gamma_g = <Bls12_381 as Pairing>::G1::normalize_batch(&powers_of_gamma_g)
        .into_iter()
        .enumerate()
        .collect();

    let neg_powers_of_h = if produce_g2_powers {
        let mut neg_powers_of_beta = vec![Fr::ONE];
        let mut cur = Fr::ONE / &beta;
        for _ in 0..max_degree {
            neg_powers_of_beta.push(cur);
            cur /= &beta;
        }

        let neg_h_table = FixedBase::get_window_table(scalar_bits, window_size, h);
        let neg_powers_of_h = FixedBase::msm::<<Bls12_381 as Pairing>::G2>(
            scalar_bits,
            window_size,
            &neg_h_table,
            &neg_powers_of_beta,
        );

        let affines = <Bls12_381 as Pairing>::G2::normalize_batch(&neg_powers_of_h);
        let mut affines_map = BTreeMap::new();
        affines.into_iter().enumerate().for_each(|(i, a)| {
            affines_map.insert(i, a);
        });
        affines_map
    } else {
        BTreeMap::new()
    };

    let h_window_size = FixedBase::get_mul_window_size(max_evals + 1);
    let h_table = FixedBase::get_window_table(scalar_bits, h_window_size, h);
    let powers_of_h = FixedBase::msm::<<Bls12_381 as Pairing>::G2>(
        scalar_bits,
        h_window_size,
        &h_table,
        &powers_of_beta,
    );
    let powers_of_h = <Bls12_381 as Pairing>::G2::normalize_batch(&powers_of_h);
    let beta_h = h.mul(beta).into_affine();
    let prepared_h = h.into();
    let prepared_beta_h = beta_h.into();
    let h = h.into_affine();

    let pp = MultiplePointUniversalParams {
        powers_of_g,
        powers_of_gamma_g,
        h,
        powers_of_h,
        neg_powers_of_h,
        prepared_h,
        prepared_beta_h,
    };
    Ok(pp)
}

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

pub fn generate_multiple_element_proof(
    element_index: &[usize],
    polynomial: &DensePolynomial<Fr>,
    domain: GeneralEvaluationDomain<Fr>,
    global_parameters: &MultiplePointUniversalParams<Bls12_381>,
) -> Result<Proof<Bls12_381>, KzgRsError> {
    // Instead of evaluating over the polynomial, we can reuse the evaluation points
    // from the rs encoding let v = polynomial.evaluate(&u);
    let x_u: DensePolynomial<Fr> = element_index
        .iter()
        .map(|&idx| {
            DensePolynomial::<Fr>::from_coefficients_vec(vec![-domain.element(idx), Fr::ONE])
        })
        .fold(
            DensePolynomial::<Fr>::from_coefficients_vec(vec![Fr::ONE]),
            |acc, poly| &acc * &poly,
        );
    let (witness_polynomial, _) = DenseOrSparsePolynomial::from(polynomial)
        .divide_with_q_and_r(&DenseOrSparsePolynomial::from(&x_u))
        .unwrap();

    let proof = commit_polynomial(
        &witness_polynomial,
        &UniversalParams {
            powers_of_g: global_parameters.powers_of_g.clone(),
            powers_of_gamma_g: global_parameters.powers_of_gamma_g.clone(),
            h: global_parameters.h,
            beta_h: global_parameters.powers_of_h[1],
            neg_powers_of_h: global_parameters.neg_powers_of_h.clone(),
            prepared_h: global_parameters.prepared_h.clone(),
            prepared_beta_h: global_parameters.prepared_beta_h.clone(),
        },
    )?;
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

pub fn interpolate<F: Field>(
    xs: &[F],
    ys: &[F],
    vanishing: &DensePolynomial<F>,
) -> DensePolynomial<F> {
    assert_eq!(xs.len(), ys.len(), "xs and ys length mismatch");
    let n = xs.len();

    let mut acc = DensePolynomial::zero();
    // Work with a DOSP so we can call divide_with_q_and_r
    let i_dosp = DenseOrSparsePolynomial::from(vanishing.clone());

    for i in 0..n {
        let xi = xs[i];
        let yi = ys[i];

        // q_i(X) = I(X) / (X - x_i)
        let lin = DensePolynomial::from_coefficients_slice(&[-xi, F::one()]); // -xi + X
        let (q_i, r) = i_dosp
            .clone()
            .divide_with_q_and_r(&DenseOrSparsePolynomial::from(lin))
            .unwrap();
        debug_assert!(r.is_zero(), "I(X) should be divisible by (X - x_i)");

        // denom_i = q_i(x_i) = ∏_{j≠i}(x_i - x_j)
        let denom_i = q_i.evaluate(&xi);
        assert!(!denom_i.is_zero(), "duplicate x_i?");

        // term_i = q_i(X) * (y_i / denom_i)
        let scale = yi * denom_i.inverse().expect("non-zero denom");
        acc = &acc + &(q_i.mul(scale));
    }

    acc
}

#[must_use]
pub fn verify_multiple_element_proof(
    element_index: &[usize],
    element: &[Fr],
    commitment: &Commitment<Bls12_381>,
    proof: &Proof<Bls12_381>,
    domain: GeneralEvaluationDomain<Fr>,
    global_parameters: &MultiplePointUniversalParams<Bls12_381>,
) -> bool {
    let x_u: DensePolynomial<Fr> = element_index
        .iter()
        .map(|&idx| {
            DensePolynomial::<Fr>::from_coefficients_vec(vec![-domain.element(idx), Fr::ONE])
        })
        .fold(
            DensePolynomial::<Fr>::from_coefficients_vec(vec![Fr::ONE]),
            |acc, poly| &acc * &poly,
        );

    let x_v = interpolate(
        &element_index
            .iter()
            .map(|&idx| domain.element(idx))
            .collect::<Vec<Fr>>(),
        element,
        &x_u,
    );

    let v_tau = commit_polynomial(
        &x_v,
        &UniversalParams {
            powers_of_g: global_parameters.powers_of_g.clone(),
            powers_of_gamma_g: global_parameters.powers_of_gamma_g.clone(),
            h: global_parameters.h,
            beta_h: global_parameters.powers_of_h[1],
            neg_powers_of_h: global_parameters.neg_powers_of_h.clone(),
            prepared_h: global_parameters.prepared_h.clone(),
            prepared_beta_h: global_parameters.prepared_beta_h.clone(),
        },
    )
    .unwrap();

    let u_tau = x_u
        .coeffs
        .iter()
        .enumerate()
        .map(|(index, &coef)| global_parameters.powers_of_h[index].mul(coef))
        .fold(G2Projective::zero(), |acc, element| acc + element);

    let commitment_check_g1 = commitment.0 + v_tau.0.neg();
    let lhs = Bls12_381::pairing(commitment_check_g1, global_parameters.h);
    let rhs = Bls12_381::pairing(proof.w, u_tau);
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
        kzg::{
            commit_polynomial, generate_element_proof, generate_multiple_element_proof,
            multiple_point_setup, verify_element_proof, verify_multiple_element_proof,
            MultiplePointUniversalParams,
        },
    };

    const COEFFICIENTS_SIZE: usize = 16;
    static GLOBAL_PARAMETERS: LazyLock<UniversalParams<Bls12_381>> = LazyLock::new(|| {
        let mut rng = thread_rng();
        KZG10::<Bls12_381, DensePolynomial<Fr>>::setup(COEFFICIENTS_SIZE - 1, true, &mut rng)
            .unwrap()
    });

    static MULTIPLE_GLOBAL_PARAMETERS: LazyLock<MultiplePointUniversalParams<Bls12_381>> =
        LazyLock::new(|| {
            let mut rng = thread_rng();
            multiple_point_setup(COEFFICIENTS_SIZE - 1, 10, true, &mut rng).unwrap()
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

        let multiple_commitment = commit_polynomial(
            &poly,
            &UniversalParams {
                powers_of_g: MULTIPLE_GLOBAL_PARAMETERS.powers_of_g.clone(),
                powers_of_gamma_g: MULTIPLE_GLOBAL_PARAMETERS.powers_of_gamma_g.clone(),
                h: MULTIPLE_GLOBAL_PARAMETERS.h,
                beta_h: MULTIPLE_GLOBAL_PARAMETERS.powers_of_h[1],
                neg_powers_of_h: MULTIPLE_GLOBAL_PARAMETERS.neg_powers_of_h.clone(),
                prepared_h: MULTIPLE_GLOBAL_PARAMETERS.prepared_h.clone(),
                prepared_beta_h: MULTIPLE_GLOBAL_PARAMETERS.prepared_beta_h.clone(),
            },
        )
            .unwrap();
        let proof = generate_multiple_element_proof(
            &(5..10).collect::<Vec<usize>>(),
            &poly,
            *DOMAIN,
            &MULTIPLE_GLOBAL_PARAMETERS,
        )
            .unwrap();
        assert!(verify_multiple_element_proof(
            &(5..10).collect::<Vec<usize>>(),
            &eval.evals[5..10],
            &multiple_commitment,
            &proof,
            *DOMAIN,
            &MULTIPLE_GLOBAL_PARAMETERS,
        ));

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