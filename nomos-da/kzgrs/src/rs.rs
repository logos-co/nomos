use std::ops::{Mul as _, Neg as _};

use ark_bls12_381::Fr;
use ark_ff::{BigInteger as _, Field as _, PrimeField as _};
use ark_poly::{
    DenseUVPolynomial as _, EvaluationDomain as _, Evaluations, GeneralEvaluationDomain,
    univariate::DensePolynomial,
};
use num_traits::Zero as _;

/// Extend a polynomial with the provided domain (size) and return
/// the original points plus the extra ones.
#[must_use]
pub fn encode(
    polynomial: &DensePolynomial<Fr>,
    domain: GeneralEvaluationDomain<Fr>,
) -> Evaluations<Fr> {
    Evaluations::from_vec_and_domain(domain.fft(&polynomial.coeffs), domain)
}

/// Interpolate points into a polynomial.
///
/// Then evaluate the polynomial in the
/// original evaluations to recover the original data.
/// `domain` need to be the same domain of the original `evaluations` and
/// `polynomial` used for encoding.
#[must_use]
pub fn decode_unchecked(
    original_chunks_len: usize,
    points: &[Option<Fr>],
    encode_domain: GeneralEvaluationDomain<Fr>,
    coeff_domain: GeneralEvaluationDomain<Fr>,
) -> Evaluations<Fr> {
    // 1) pick the available points from the ENCODE domain
    let (mut points, mut roots_of_unity): (Vec<Fr>, Vec<Fr>) = points
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.map(|e| (e, encode_domain.element(i))))
        .unzip();

    // 2) truncate enough points
    points.truncate(original_chunks_len);
    roots_of_unity.truncate(original_chunks_len);

    // 3) interpolate polynomial
    let coeffs = lagrange_interpolate(&points, &roots_of_unity);

    // 4) NOW evaluate on the **original** domain, not the encode domain
    let evals_on_original = coeff_domain.fft(&coeffs);

    Evaluations::from_vec_and_domain(evals_on_original, coeff_domain)
}

/// Interpolate a set of points using lagrange interpolation and roots of unity
///
/// Warning!! Be aware that the mapping between points and roots of unity is the
/// intended: A polynomial `f(x)` is derived for `w_x` (root) mapping to `p_x`.
/// `[(w_1, p_1)..(w_n, p_n)]` even if points are missing it is important to
/// keep the mapping integrity.
#[must_use]
pub fn lagrange_interpolate(points: &[Fr], roots_of_unity: &[Fr]) -> DensePolynomial<Fr> {
    assert_eq!(points.len(), roots_of_unity.len());
    let mut result = DensePolynomial::from_coefficients_vec(vec![Fr::zero()]);
    for i in 0..roots_of_unity.len() {
        let mut summand = DensePolynomial::from_coefficients_vec(vec![points[i]]);
        for j in 0..points.len() {
            if i != j {
                let weight_adjustment =
                    (roots_of_unity[i] - roots_of_unity[j])
                        .inverse()
                        .expect(
                            "Roots of unity are/should not repeated. If this panics it means we have no coefficients enough in the evaluation domain"
                        );
                summand = summand.naive_mul(&DensePolynomial::from_coefficients_vec(vec![
                    weight_adjustment.mul(roots_of_unity[j]).neg(),
                    weight_adjustment,
                ]));
            }
        }
        result = result + summand;
    }
    result
}

/// Reconstruct bytes from the polynomial evaluation points using original chunk
/// size and a set of points
pub fn points_to_bytes<const CHUNK_SIZE: usize>(points: &[Fr]) -> Vec<u8> {
    fn point_to_buff<const CHUNK_SIZE: usize>(p: &Fr) -> impl Iterator<Item = u8> {
        p.into_bigint().to_bytes_le().into_iter().take(CHUNK_SIZE)
    }
    points
        .iter()
        .flat_map(point_to_buff::<CHUNK_SIZE>)
        .collect()
}

#[cfg(test)]
mod test {
    use std::sync::LazyLock;

    use ark_bls12_381::Fr;
    use ark_poly::{EvaluationDomain as _, GeneralEvaluationDomain};
    use rand::{Fill as _, thread_rng};

    use crate::{
        common::bytes_to_polynomial,
        rs::{decode_unchecked, encode, points_to_bytes},
    };

    const ENCODE_DOMAIN_SIZE: usize = 32;

    static ENCODE_DOMAIN: LazyLock<GeneralEvaluationDomain<Fr>> =
        LazyLock::new(|| GeneralEvaluationDomain::new(ENCODE_DOMAIN_SIZE).unwrap());

    static COEFF_DOMAIN: LazyLock<GeneralEvaluationDomain<Fr>> = LazyLock::new(|| {
        // we want “half” of the encode domain size
        let half = ENCODE_DOMAIN_SIZE / 2;
        GeneralEvaluationDomain::new(half).unwrap()
    });

    #[test]
    fn test_encode_decode() {
        let mut bytes: [u8; 496] = [0; 496];
        let mut rng = thread_rng();
        bytes.try_fill(&mut rng).unwrap();

        let (_evals, poly) = bytes_to_polynomial::<31>(&bytes, *COEFF_DOMAIN).unwrap();
        let encoded = encode(&poly, *ENCODE_DOMAIN);
        let mut encoded: Vec<Option<Fr>> = encoded.evals.into_iter().map(Some).collect();
        let decoded = decode_unchecked(16, &encoded, *ENCODE_DOMAIN, *COEFF_DOMAIN);
        let decoded_bytes = points_to_bytes::<31>(&decoded.evals);
        assert_eq!(decoded_bytes, bytes);

        // check with missing pieces
        for i in (1..encoded.len()).step_by(2) {
            encoded[i] = None;
        }

        let decoded_missing = decode_unchecked(16, &encoded, *ENCODE_DOMAIN, *COEFF_DOMAIN);
        let decoded_bytes = points_to_bytes::<31>(&decoded_missing.evals);
        assert_eq!(decoded_bytes, bytes);
    }
}
