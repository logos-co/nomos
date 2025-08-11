#[cfg(feature = "deser")]
pub mod deserialize;

#[cfg(feature = "deser")]
use ark_bn254::Bn254;
#[cfg(feature = "deser")]
use ark_bn254::{G1Affine, G2Affine};
use ark_ec::pairing::Pairing;

#[cfg(feature = "deser")]
pub use crate::proof::deserialize::ProofJsonDeser;
#[cfg(feature = "deser")]
use crate::protocol::Protocol;
#[cfg(feature = "deser")]
use crate::utils::*;

pub struct Proof<E: Pairing> {
    pi_a: E::G1Affine,
    pi_b: E::G2Affine,
    pi_c: E::G1Affine,
}

impl<E: Pairing> From<&Proof<E>> for ark_groth16::Proof<E> {
    fn from(value: &Proof<E>) -> Self {
        let Proof { pi_a, pi_b, pi_c } = value;
        Self {
            a: *pi_a,
            b: *pi_b,
            c: *pi_c,
        }
    }
}

#[cfg(feature = "deser")]
#[derive(Debug, thiserror::Error)]
pub enum FromJsonError {
    #[error("invalid protocol: {0}, expected: \"groth16\"")]
    WrongProtocol(String),
    #[error("could not parse G1 point, due to: {0:?}")]
    G1PointConversionError(<G1Affine as TryFrom<StringifiedG1>>::Error),
    #[error("could not parse G2 point, due to: {0:?}")]
    G2PointConversionError(<G2Affine as TryFrom<StringifiedG2>>::Error),
}
#[cfg(feature = "deser")]
impl TryFrom<ProofJsonDeser> for Proof<Bn254> {
    type Error = FromJsonError;
    fn try_from(value: ProofJsonDeser) -> Result<Self, Self::Error> {
        if value.protocol != Protocol::Groth16 {
            return Err(FromJsonError::WrongProtocol(
                value.protocol.as_ref().to_string(),
            ));
        }
        let ProofJsonDeser {
            pi_a, pi_b, pi_c, ..
        } = value;
        let pi_a = StringifiedG1(pi_a)
            .try_into()
            .map_err(FromJsonError::G1PointConversionError)?;
        let pi_b = StringifiedG2(pi_b)
            .try_into()
            .map_err(FromJsonError::G2PointConversionError)?;
        let pi_c = StringifiedG1(pi_c)
            .try_into()
            .map_err(FromJsonError::G1PointConversionError)?;

        Ok(Self { pi_b, pi_c, pi_a })
    }
}
