#[cfg(feature = "deser")]
pub mod deserialize;

#[cfg(feature = "deser")]
use ark_bn254::Bn254;
use ark_ec::pairing::Pairing;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize as _, SerializationError};

#[cfg(feature = "deser")]
use crate::from_json_error::FromJsonError;
#[cfg(feature = "deser")]
pub use crate::proof::deserialize::ProofJsonDeser;
#[cfg(feature = "deser")]
use crate::protocol::Protocol;
#[cfg(feature = "deser")]
use crate::utils::{JsonG1, JsonG2, StringifiedG1, StringifiedG2};

pub struct Proof<E: Pairing> {
    pi_a: E::G1Affine,
    pi_b: E::G2Affine,
    pi_c: E::G1Affine,
}

pub struct CompressedProof {
    pi_a: Vec<u8>,
    pi_b: Vec<u8>,
    pi_c: Vec<u8>,
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
impl TryFrom<ProofJsonDeser> for Proof<Bn254> {
    type Error = FromJsonError;
    fn try_from(value: ProofJsonDeser) -> Result<Self, Self::Error> {
        if !matches!(value.protocol, Protocol::Groth16) {
            return Err(Self::Error::WrongProtocol(
                value.protocol.as_ref().to_owned(),
            ));
        }
        let ProofJsonDeser {
            pi_a, pi_b, pi_c, ..
        } = value;
        let pi_a = StringifiedG1(pi_a)
            .try_into()
            .map_err(Self::Error::G1PointConversionError)?;
        let pi_b = StringifiedG2(pi_b)
            .try_into()
            .map_err(Self::Error::G2PointConversionError)?;
        let pi_c = StringifiedG1(pi_c)
            .try_into()
            .map_err(Self::Error::G1PointConversionError)?;

        Ok(Self { pi_a, pi_b, pi_c })
    }
}

impl<E: Pairing> TryFrom<&Proof<E>> for CompressedProof {
    type Error = SerializationError;
    fn try_from(value: &Proof<E>) -> Result<Self, SerializationError> {
        let Proof { pi_a, pi_b, pi_c } = value;
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut c = Vec::new();
        pi_a.serialize_compressed(&mut a)?;
        pi_b.serialize_compressed(&mut b)?;
        pi_c.serialize_compressed(&mut c)?;
        Ok(Self {
            pi_a: a,
            pi_b: b,
            pi_c: c,
        })
    }
}

impl<E: Pairing> TryFrom<&CompressedProof> for Proof<E> {
    type Error = SerializationError;
    fn try_from(value: &CompressedProof) -> Result<Self, SerializationError> {
        let CompressedProof { pi_a, pi_b, pi_c } = value;
        let a = <E::G1Affine as CanonicalDeserialize>::deserialize_compressed(&pi_a[..])?;
        let b = <E::G2Affine as CanonicalDeserialize>::deserialize_compressed(&pi_b[..])?;
        let c = <E::G1Affine as CanonicalDeserialize>::deserialize_compressed(&pi_c[..])?;
        Ok(Self {
            pi_a: a,
            pi_b: b,
            pi_c: c,
        })
    }
}
