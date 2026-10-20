pub mod deserialize;
use ark_ec::{AffineRepr as _, pairing::Pairing};
use ark_serialize::{CanonicalDeserialize as _, CanonicalSerialize as _};
pub use deserialize::VerificationKeyJsonDeser;

use crate::{
    from_json_error::FromJsonError,
    protocol::Protocol,
    utils::{StringifiedG1, StringifiedG2},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationKey<E: Pairing> {
    pub alpha_1: E::G1Affine,
    pub beta_2: E::G2Affine,
    pub gamma_2: E::G2Affine,
    pub delta_2: E::G2Affine,
    pub ic: Vec<E::G1Affine>,
}
impl TryFrom<VerificationKeyJsonDeser> for VerificationKey<ark_bn254::Bn254> {
    type Error = FromJsonError;
    fn try_from(value: VerificationKeyJsonDeser) -> Result<Self, Self::Error> {
        if !matches!(value.protocol, Protocol::Groth16) {
            return Err(Self::Error::WrongProtocol(
                value.protocol.as_ref().to_owned(),
            ));
        }
        let VerificationKeyJsonDeser {
            alpha_1,
            beta_2,
            gamma_2,
            delta2,
            ic,
            ..
        } = value;
        let alpha_1 = StringifiedG1(alpha_1)
            .try_into()
            .map_err(Self::Error::G1PointConversionError)?;
        let beta_2 = StringifiedG2(beta_2)
            .try_into()
            .map_err(Self::Error::G2PointConversionError)?;
        let gamma_2 = StringifiedG2(gamma_2)
            .try_into()
            .map_err(Self::Error::G2PointConversionError)?;
        let delta_2 = StringifiedG2(delta2)
            .try_into()
            .map_err(Self::Error::G2PointConversionError)?;
        let ic: Vec<ark_bn254::G1Affine> = ic
            .into_iter()
            .map(StringifiedG1)
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Self::Error::G1PointConversionError)?;

        Ok(Self {
            alpha_1,
            beta_2,
            gamma_2,
            delta_2,
            ic,
        })
    }
}
pub struct PreparedVerificationKey<E: Pairing> {
    vk: ark_groth16::PreparedVerifyingKey<E>,
}

impl<E: Pairing> PreparedVerificationKey<E> {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        self.vk
            .serialize_compressed(&mut bytes)
            .expect("vk failed to serialize");

        bytes
    }

    pub const fn alpha_g1(&self) -> &E::G1Affine {
        &self.vk.vk.alpha_g1
    }

    pub const fn beta_g2(&self) -> &E::G2Affine {
        &self.vk.vk.beta_g2
    }

    pub const fn delta_g2(&self) -> &E::G2Affine {
        &self.vk.vk.delta_g2
    }

    pub const fn gamma_abc_g1(&self) -> &Vec<E::G1Affine> {
        &self.vk.vk.gamma_abc_g1
    }

    pub const fn alpha_g1_beta_g2(&self) -> &E::TargetField {
        &self.vk.alpha_g1_beta_g2
    }

    pub const fn gamma_g2_neg_pc(&self) -> &E::G2Prepared {
        &self.vk.gamma_g2_neg_pc
    }

    pub const fn delta_g2_neg_pc(&self) -> &E::G2Prepared {
        &self.vk.delta_g2_neg_pc
    }
}

impl<E: Pairing> From<VerificationKey<E>> for ark_groth16::VerifyingKey<E> {
    fn from(value: VerificationKey<E>) -> Self {
        let VerificationKey {
            alpha_1,
            beta_2,
            gamma_2,
            delta_2,
            ic,
        } = value;

        Self {
            alpha_g1: alpha_1,
            beta_g2: beta_2,
            gamma_g2: gamma_2,
            delta_g2: delta_2,
            gamma_abc_g1: ic,
        }
    }
}

impl<E: Pairing> VerificationKey<E> {
    pub fn into_prepared(self) -> PreparedVerificationKey<E> {
        let vk: ark_groth16::VerifyingKey<E> = self.into();
        PreparedVerificationKey { vk: vk.into() }
    }
}

pub const COMPRESSED_G1_SIZE: usize = 32;
pub const COMPRESSED_G2_SIZE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPoint {
    Alpha,
    Beta,
    Gamma,
    Delta,
    Ic(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyPointError {
    #[error("{0:?} is not a compressed point of its prime order subgroup")]
    Invalid(KeyPoint),
    #[error("{0:?} is the point at infinity")]
    Infinity(KeyPoint),
}

pub fn g1_from_compressed(
    bytes: &[u8; COMPRESSED_G1_SIZE],
    point: KeyPoint,
) -> Result<ark_bn254::G1Affine, KeyPointError> {
    let decoded = ark_bn254::G1Affine::deserialize_compressed(bytes.as_slice())
        .map_err(|_| KeyPointError::Invalid(point))?;
    if decoded.is_zero() {
        return Err(KeyPointError::Infinity(point));
    }
    Ok(decoded)
}

pub fn g2_from_compressed(
    bytes: &[u8; COMPRESSED_G2_SIZE],
    point: KeyPoint,
) -> Result<ark_bn254::G2Affine, KeyPointError> {
    let decoded = ark_bn254::G2Affine::deserialize_compressed(bytes.as_slice())
        .map_err(|_| KeyPointError::Invalid(point))?;
    if decoded.is_zero() {
        return Err(KeyPointError::Infinity(point));
    }
    Ok(decoded)
}

#[must_use]
pub fn g1_to_compressed(point: &ark_bn254::G1Affine) -> [u8; COMPRESSED_G1_SIZE] {
    let mut bytes = [0u8; COMPRESSED_G1_SIZE];
    point
        .serialize_compressed(bytes.as_mut_slice())
        .expect("a point fits its compressed size");
    bytes
}

#[must_use]
pub fn g2_to_compressed(point: &ark_bn254::G2Affine) -> [u8; COMPRESSED_G2_SIZE] {
    let mut bytes = [0u8; COMPRESSED_G2_SIZE];
    point
        .serialize_compressed(bytes.as_mut_slice())
        .expect("a point fits its compressed size");
    bytes
}

impl VerificationKey<ark_bn254::Bn254> {
    #[must_use]
    pub fn alpha_beta(&self) -> ark_bn254::Fq12 {
        ark_bn254::Bn254::pairing(self.alpha_1, self.beta_2).0
    }

    // The line coefficients of `-γ` and `-δ` are computed here, `e(α, β)` comes
    // from the ledger.
    #[must_use]
    #[expect(
        clippy::large_types_passed_by_value,
        reason = "moved into the prepared key"
    )]
    pub fn prepare_with_alpha_beta(
        &self,
        alpha_beta: ark_bn254::Fq12,
    ) -> PreparedVerificationKey<ark_bn254::Bn254> {
        PreparedVerificationKey {
            vk: ark_groth16::PreparedVerifyingKey {
                vk: self.clone().into(),
                alpha_g1_beta_g2: alpha_beta,
                gamma_g2_neg_pc: (-self.gamma_2).into(),
                delta_g2_neg_pc: (-self.delta_2).into(),
            },
        }
    }
}

impl<E: Pairing> AsRef<ark_groth16::PreparedVerifyingKey<E>> for PreparedVerificationKey<E> {
    fn as_ref(&self) -> &ark_groth16::PreparedVerifyingKey<E> {
        &self.vk
    }
}

#[cfg(test)]
mod tests {
    use ark_bn254::{Fr, G1Affine, G2Affine};
    use ark_ec::AffineRepr as _;

    use super::{
        KeyPoint, KeyPointError, g1_from_compressed, g1_to_compressed, g2_from_compressed,
        g2_to_compressed,
    };
    use crate::{
        groth16_verify,
        test_utils::{off_subgroup_g2_compressed, three_input_prove, three_input_setup},
    };

    #[test]
    fn compressed_points_round_trip_and_reject_infinity() {
        let (key, _) = three_input_setup(1);
        assert_eq!(
            g1_from_compressed(&g1_to_compressed(&key.alpha_1), KeyPoint::Alpha),
            Ok(key.alpha_1)
        );
        assert_eq!(
            g2_from_compressed(&g2_to_compressed(&key.beta_2), KeyPoint::Beta),
            Ok(key.beta_2)
        );

        // arkworks flags the point at infinity with bit 6 of the last byte
        let mut infinity_g1 = [0u8; 32];
        infinity_g1[31] = 0x40;
        let mut infinity_g2 = [0u8; 64];
        infinity_g2[63] = 0x40;
        assert_eq!(g1_to_compressed(&G1Affine::zero()), infinity_g1);
        assert_eq!(g2_to_compressed(&G2Affine::zero()), infinity_g2);
        assert_eq!(
            g1_from_compressed(&infinity_g1, KeyPoint::Ic(2)),
            Err(KeyPointError::Infinity(KeyPoint::Ic(2)))
        );
        assert_eq!(
            g2_from_compressed(&infinity_g2, KeyPoint::Delta),
            Err(KeyPointError::Infinity(KeyPoint::Delta))
        );
    }

    #[test]
    fn compressed_points_reject_bytes_above_the_base_field() {
        // The placeholder bytes of the spec's PROGRAM_REGISTER vector: x is not below
        // q.
        assert_eq!(
            g1_from_compressed(&[0x3e; 32], KeyPoint::Alpha),
            Err(KeyPointError::Invalid(KeyPoint::Alpha))
        );
        assert_eq!(
            g2_from_compressed(&[0x3f; 64], KeyPoint::Beta),
            Err(KeyPointError::Invalid(KeyPoint::Beta))
        );
    }

    #[test]
    fn g2_points_outside_the_prime_order_subgroup_are_rejected() {
        assert_eq!(
            g2_from_compressed(&off_subgroup_g2_compressed(), KeyPoint::Gamma),
            Err(KeyPointError::Invalid(KeyPoint::Gamma))
        );
    }

    #[test]
    fn a_key_prepared_from_its_stored_alpha_beta_verifies() {
        let (key, proving_key) = three_input_setup(1);
        let inputs = [Fr::from(3u64), Fr::from(5u64), Fr::from(7u64)];
        let proof = three_input_prove(&proving_key, inputs, 2);
        let alpha_beta = key.alpha_beta();
        assert_eq!(&alpha_beta, key.clone().into_prepared().alpha_g1_beta_g2());
        let prepared = key.prepare_with_alpha_beta(alpha_beta);
        assert!(groth16_verify(&prepared, &proof, &inputs).unwrap());
        assert!(
            !groth16_verify(
                &prepared,
                &proof,
                &[Fr::from(3u64), Fr::from(5u64), Fr::from(8u64)]
            )
            .unwrap()
        );
    }
}
