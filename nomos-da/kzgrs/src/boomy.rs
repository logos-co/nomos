use std::ops::{Deref, Mul as _, Neg as _};

use ark_bls12_381::{Bls12_381, Fr, G1Projective};
use ark_ec::{pairing::Pairing, scalar_mul::fixed_base::FixedBase, CurveGroup, VariableBaseMSM};
use ark_ff::{Field, PrimeField, UniformRand};
use ark_poly::{
    univariate, DenseUVPolynomial, EvaluationDomain, GeneralEvaluationDomain, Polynomial,
};
use ark_poly_commit::{
    kzg10::{Commitment, Powers, Proof, UniversalParams, KZG10},
    Error,
};
use ark_std::rand::RngCore;
use num_bigint::BigUint;
use num_traits::{One as _, Zero as _};

use crate::{bivariate::DensePolynomial, common::KzgRsError};

pub struct BivariateUniversalParams<E: Pairing> {
    /// row-wise matrix for exponent values of (\tau_x^i * \tau_y^j)
    /// where i \in [0, d_x], j \in [0, d_y], and \tau_x, \tau_y are two
    /// trapdoors for the two variables X and Y.
    ///
    /// [  X^0 * Y^0   X^0 * Y^1   X^0 * Y^2   ...   X^0 * Y^d_y  ]
    /// [  X^1 * Y^0   X^1 * Y^1   X^1 * Y^2   ...   X^1 * Y^d_y  ]
    /// [  X^2 * Y^0   X^2 * Y^1   X^2 * Y^2   ...   X^2 * Y^d_y  ]
    /// [     ...          ...          ...    ...       ...      ]
    /// [ X^d_x * Y^0  X^d_x * Y^1  X^d_x * Y^2  ...  X^d_x * Y^d_y ]
    /// Replace X with \tau_x, Y with \tau_y
    pub powers_of_g: Vec<Vec<E::G1Affine>>,
    /// The generator of G2.
    pub h: E::G2Affine,
    /// \beta_1^i times the above generator of G2, where `i` ranges from 0 to
    /// `max_points`.
    pub beta_2_h: E::G2Affine,
    /// The generator of G2, prepared for use in pairings.
    pub prepared_h: E::G2Prepared,
}

pub fn bivariate_setup<R: RngCore>(
    max_x_degree: usize,
    max_y_degree: usize,
    rng: &mut R,
) -> Result<BivariateUniversalParams<Bls12_381>, Error> {
    if (max_x_degree < 1) || (max_y_degree < 1) {
        return Err(Error::DegreeIsZero);
    }
    let beta_1 = Fr::rand(rng);
    let beta_2 = Fr::rand(rng);
    let g = <Bls12_381 as Pairing>::G1::rand(rng);
    let h = <Bls12_381 as Pairing>::G2::rand(rng);

    let mut powers_of_beta: Vec<Vec<Fr>> = Vec::new();

    let mut cur_1 = Fr::ONE;
    for i in 0..max_x_degree + 1 {
        powers_of_beta.push(vec![cur_1]);
        let mut cur_2 = beta_2.clone();
        for _ in 0..max_y_degree {
            powers_of_beta[i].push(cur_1 * cur_2);
            cur_2 *= &beta_2;
        }
        cur_1 *= &beta_1;
    }

    let window_size = FixedBase::get_mul_window_size(max_y_degree + 1);
    let scalar_bits = Fr::MODULUS_BIT_SIZE as usize;
    let g_table = FixedBase::get_window_table(scalar_bits, window_size, g);
    let powers_of_g: Vec<_> = powers_of_beta
        .iter()
        .map(|powers| {
            <Bls12_381 as Pairing>::G1::normalize_batch(
                &FixedBase::msm::<<Bls12_381 as Pairing>::G1>(
                    scalar_bits,
                    window_size,
                    &g_table,
                    powers,
                ),
            )
        })
        .collect();

    let beta_2_h = h.mul(&beta_2).into_affine();

    let prepared_h = h.into();
    let h = h.into_affine();

    let pp = BivariateUniversalParams {
        powers_of_g,
        h,
        beta_2_h,
        prepared_h,
    };
    Ok(pp)
}

/// Commit to a polynomial where each of the evaluations are over `w(i)` for the
/// degree of the polynomial being omega (`w`) the root of unity (2^x).
pub fn commit_bivariate_polynomial(
    polynomial: &DensePolynomial<Fr>,
    global_parameters: &BivariateUniversalParams<Bls12_381>,
) -> Commitment<Bls12_381> {
    let bases: Vec<_> = global_parameters
        .powers_of_g
        .iter()
        .zip(polynomial.coeffs.iter())
        .flat_map(|(b_row, s_row)| b_row.iter().take(s_row.len()))
        .copied()
        .collect();
    let scalars: Vec<Fr> = polynomial
        .coeffs
        .iter()
        .flat_map(|s_row| s_row.iter().copied())
        .collect();
    Commitment(G1Projective::msm(&bases, &scalars).unwrap().into_affine())
}

pub fn commit_unvivariate_polynomial_in_y(
    polynomial: &univariate::DensePolynomial<Fr>,
    global_parameters: &BivariateUniversalParams<Bls12_381>,
) -> Commitment<Bls12_381> {
    let bases = global_parameters
        .powers_of_g
        .get(0)
        .expect("bivariate setup must contain at least one row (Y^0)");
    Commitment(
        G1Projective::msm(&bases[..polynomial.coeffs.len()], &polynomial.coeffs)
            .unwrap()
            .into_affine(),
    )
}

pub fn commit_unvivariate_polynomial_in_x(
    polynomial: &univariate::DensePolynomial<Fr>,
    global_parameters: &BivariateUniversalParams<Bls12_381>,
) -> Commitment<Bls12_381> {
    let bases = global_parameters
        .powers_of_g
        .iter()
        .map(|vec| vec[0])
        .collect::<Vec<_>>();
    Commitment(
        G1Projective::msm(&bases[..polynomial.coeffs.len()], &polynomial.coeffs)
            .unwrap()
            .into_affine(),
    )
}

pub fn generate_bivariate_proof(
    // The column of the sample:
    sample_index: usize,
    polynomial: &DensePolynomial<Fr>,
    y_domain: GeneralEvaluationDomain<Fr>,
    global_parameters: &BivariateUniversalParams<Bls12_381>,
) -> Result<Proof<Bls12_381>, KzgRsError> {
    let (witness_polynomial, _) = polynomial
        .divide_with_q_and_r(
            &univariate::DensePolynomial::from_coefficients_vec(vec![
                -y_domain.element(sample_index),
                Fr::ONE,
            ]),
            false,
        )
        .unwrap();

    let proof = commit_bivariate_polynomial(&witness_polynomial, &global_parameters);

    let proof = Proof {
        w: proof.0,
        random_v: None,
    };
    Ok(proof)
}

pub fn interpolate<F: Field>(
    xs: &[F],
    ys: &[F],
    vanishing: &univariate::DensePolynomial<F>,
) -> univariate::DensePolynomial<F> {
    assert_eq!(xs.len(), ys.len(), "xs and ys length mismatch");
    let n = xs.len();

    let mut acc = univariate::DensePolynomial::zero();
    // Work with a DOSP so we can call divide_with_q_and_r
    let i_dosp = univariate::DenseOrSparsePolynomial::from(vanishing.clone());

    for i in 0..n {
        let xi = xs[i];
        let yi = ys[i];

        // q_i(X) = I(X) / (X - x_i)
        let lin = univariate::DensePolynomial::from_coefficients_slice(&[-xi, F::one()]); // -xi + X
        let (q_i, r) = i_dosp
            .clone()
            .divide_with_q_and_r(&univariate::DenseOrSparsePolynomial::from(lin))
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
pub fn verify_bivariate_proof(
    sample_index: usize,
    elements: &[Fr],
    commitment: &Commitment<Bls12_381>,
    proof: &Proof<Bls12_381>,
    x_domain: GeneralEvaluationDomain<Fr>,
    y_domain: GeneralEvaluationDomain<Fr>,
    global_parameters: &BivariateUniversalParams<Bls12_381>,
) -> bool {
    let x_u: univariate::DensePolynomial<Fr> = elements
        .iter()
        .enumerate()
        .map(|(idx, element)| {
            univariate::DensePolynomial::<Fr>::from_coefficients_vec(vec![
                -x_domain.element(idx),
                Fr::ONE,
            ])
        })
        .fold(
            univariate::DensePolynomial::<Fr>::from_coefficients_vec(vec![Fr::ONE]),
            |acc, poly| &acc * &poly,
        );

    let x_v = interpolate(
        &(0..elements.len())
            .collect::<Vec<usize>>()
            .iter()
            .map(|&idx| x_domain.element(idx))
            .collect::<Vec<Fr>>(),
        elements,
        &x_u,
    );

    let v_tau = commit_unvivariate_polynomial_in_x(&x_v, &global_parameters);

    let commitment_check_g1 = commitment.0 + v_tau.0.neg();
    let evaluation_check_g2 =
        global_parameters.beta_2_h + global_parameters.h.mul(y_domain.element(sample_index)).neg();
    let lhs = Bls12_381::pairing(commitment_check_g1, global_parameters.h);
    let rhs = Bls12_381::pairing(proof.w, evaluation_check_g2);
    lhs == rhs
}

#[cfg(test)]
mod test {
    use std::sync::LazyLock;

    use ark_bls12_381::{Bls12_381, Fr};
    use ark_poly::{
        univariate, DenseUVPolynomial as _, EvaluationDomain as _, GeneralEvaluationDomain,
        Polynomial,
    };
    use ark_poly_commit::kzg10::{UniversalParams, KZG10};
    use rand::{random, seq::index::sample, thread_rng, Fill as _};
    use rayon::{
        iter::{IndexedParallelIterator as _, ParallelIterator as _},
        prelude::IntoParallelRefIterator as _,
    };

    use crate::{
        bivariate::DensePolynomial,
        boomy::{
            bivariate_setup, commit_bivariate_polynomial, generate_bivariate_proof,
            verify_bivariate_proof, BivariateUniversalParams,
        },
        common::bytes_to_polynomial,
        kzg::{
            commit_polynomial, generate_element_proof, generate_multiple_element_proof,
            multiple_point_setup, verify_element_proof, verify_multiple_element_proof,
            MultiplePointUniversalParams,
        },
    };

    const NUMBER_OF_SUBNETWORK: usize = 10;
    const SAMPLE_SIZE: usize = 5;
    static GLOBAL_PARAMETERS: LazyLock<BivariateUniversalParams<Bls12_381>> = LazyLock::new(|| {
        let mut rng = thread_rng();
        bivariate_setup(SAMPLE_SIZE - 1, NUMBER_OF_SUBNETWORK - 1, &mut rng).unwrap()
    });

    static X_DOMAIN: LazyLock<GeneralEvaluationDomain<Fr>> =
        LazyLock::new(|| GeneralEvaluationDomain::new(SAMPLE_SIZE).unwrap());

    static Y_DOMAIN: LazyLock<GeneralEvaluationDomain<Fr>> =
        LazyLock::new(|| GeneralEvaluationDomain::new(NUMBER_OF_SUBNETWORK).unwrap());

    #[test]
    fn generate_proof_and_validate() {
        let mut rng = thread_rng();
        let poly = DensePolynomial::rand(SAMPLE_SIZE - 1, NUMBER_OF_SUBNETWORK - 1, &mut rng);
        let commitment = commit_bivariate_polynomial(&poly, &GLOBAL_PARAMETERS);
        let sample_index: usize = (random::<u64>() % NUMBER_OF_SUBNETWORK as u64) as usize;
        let sample = (0..SAMPLE_SIZE)
            .collect::<Vec<usize>>()
            .iter()
            .map(|&i| poly.evaluate(&(X_DOMAIN.element(i), Y_DOMAIN.element(sample_index))))
            .collect::<Vec<Fr>>();

        let proof =
            generate_bivariate_proof(sample_index, &poly, *Y_DOMAIN, &GLOBAL_PARAMETERS).unwrap();
        assert!(verify_bivariate_proof(
            sample_index,
            &sample,
            &commitment,
            &proof,
            *X_DOMAIN,
            *Y_DOMAIN,
            &GLOBAL_PARAMETERS,
        ));

        /*eval.evals
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
        });*/
    }
}
