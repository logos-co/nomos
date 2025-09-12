#[cfg(feature = "deser")]
pub mod deserialize;

use std::marker::PhantomData;

#[cfg(feature = "deser")]
use ark_bn254::Bn254;
use ark_ec::pairing::Pairing;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize as _, SerializationError};
use generic_array::{ArrayLength, GenericArray};

#[cfg(feature = "deser")]
use crate::from_json_error::FromJsonError;
#[cfg(feature = "deser")]
pub use crate::proof::deserialize::ProofJsonDeser;
#[cfg(feature = "deser")]
use crate::protocol::Protocol;
#[cfg(feature = "deser")]
use crate::utils::{JsonG1, JsonG2, StringifiedG1, StringifiedG2};

#[derive(Clone, Debug)]
pub struct Proof<E: Pairing> {
    pub pi_a: E::G1Affine,
    pub pi_b: E::G2Affine,
    pub pi_c: E::G1Affine,
}

pub trait CompressSize: Pairing {
    type G1CompressedSize: ArrayLength;
    type G2CompressedSize: ArrayLength;
}

#[derive(Clone, Debug)]
pub struct CompressedProof<E: CompressSize> {
    pub pi_a: GenericArray<u8, E::G1CompressedSize>,
    pub pi_b: GenericArray<u8, E::G2CompressedSize>,
    pub pi_c: GenericArray<u8, E::G1CompressedSize>,
    _pairing: PhantomData<E>,
}

impl<E: CompressSize> CompressedProof<E> {
    pub const fn new(
        pi_a: GenericArray<u8, E::G1CompressedSize>,
        pi_b: GenericArray<u8, E::G2CompressedSize>,
        pi_c: GenericArray<u8, E::G1CompressedSize>,
    ) -> Self {
        Self {
            pi_a,
            pi_b,
            pi_c,
            _pairing: PhantomData,
        }
    }
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

impl<E: Pairing + CompressSize> TryFrom<&Proof<E>> for CompressedProof<E> {
    type Error = SerializationError;
    fn try_from(value: &Proof<E>) -> Result<Self, SerializationError> {
        let Proof { pi_a, pi_b, pi_c } = value;
        let mut a = GenericArray::default();
        let mut b = GenericArray::default();
        let mut c = GenericArray::default();
        pi_a.serialize_compressed(a.as_mut_slice())?;
        pi_b.serialize_compressed(b.as_mut_slice())?;
        pi_c.serialize_compressed(c.as_mut_slice())?;
        Ok(Self {
            pi_a: a,
            pi_b: b,
            pi_c: c,
            _pairing: PhantomData,
        })
    }
}

impl<E: Pairing + CompressSize> TryFrom<&CompressedProof<E>> for Proof<E> {
    type Error = SerializationError;
    fn try_from(value: &CompressedProof<E>) -> Result<Self, SerializationError> {
        let CompressedProof {
            pi_a, pi_b, pi_c, ..
        } = value;
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
