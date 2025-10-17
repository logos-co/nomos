#[cfg(feature = "deser")]
pub mod deserialize;

use ark_bn254::{Bn254, G1Affine, G2Affine};
use ark_ec::pairing::Pairing;
#[cfg(feature = "deser")]
pub use deserialize::VerificationKeyJsonDeser;

#[cfg(feature = "deser")]
use crate::from_json_error::FromJsonError;
#[cfg(feature = "deser")]
use crate::protocol::Protocol;
#[cfg(feature = "deser")]
use crate::utils::{StringifiedG1, StringifiedG2};

#[derive(Eq, PartialEq)]
pub struct VerificationKey<E: Pairing> {
    pub alpha_1: E::G1Affine,
    pub beta_2: E::G2Affine,
    pub gamma_2: E::G2Affine,
    pub delta_2: E::G2Affine,
    pub ic: Vec<E::G1Affine>,
}
#[cfg(feature = "deser")]
impl TryFrom<VerificationKeyJsonDeser> for VerificationKey<Bn254> {
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
        let ic: Vec<G1Affine> = ic
            .into_iter()
            .map(StringifiedG1)
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Self::Error::G2PointConversionError)?;

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

impl<E: Pairing> AsRef<ark_groth16::PreparedVerifyingKey<E>> for PreparedVerificationKey<E> {
    fn as_ref(&self) -> &ark_groth16::PreparedVerifyingKey<E> {
        &self.vk
    }
}

impl PreparedVerificationKey<Bn254> {
    pub const fn alpha_g1(&self) -> &G1Affine {
        &self.vk.vk.alpha_g1
    }

    pub const fn beta_g2(&self) -> &G2Affine {
        &self.vk.vk.beta_g2
    }

    pub const fn delta_g2(&self) -> &G2Affine {
        &self.vk.vk.delta_g2
    }

    pub const fn gamma_abc_g1(&self) -> &Vec<G1Affine> {
        &self.vk.vk.gamma_abc_g1
    }

    pub const fn alpha_g1_beta_g2(&self) -> &<Bn254 as Pairing>::TargetField {
        &self.vk.alpha_g1_beta_g2
    }

    pub const fn gamma_g2_neg_pc(&self) -> &<Bn254 as Pairing>::G2Prepared {
        &self.vk.gamma_g2_neg_pc
    }

    pub const fn delta_g2_neg_pc(&self) -> &<Bn254 as Pairing>::G2Prepared {
        &self.vk.delta_g2_neg_pc
    }
}
