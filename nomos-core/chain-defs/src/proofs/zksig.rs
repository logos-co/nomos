use blake2::digest::{Update as _, VariableOutput as _};
use groth16::{Fr, fr_to_bytes, serde::serde_fr};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeAs, SerializeAs, serde_as};

use crate::utils::serde_big_bytes_newtype;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DummyZkSignature([u8; 128]);

serde_big_bytes_newtype!(DummyZkSignature, 128);

impl DummyZkSignature {
    #[must_use]
    pub fn prove(public_inputs: &ZkSignaturePublic) -> Self {
        let mut hasher = blake2::Blake2bVar::new(128).unwrap();
        hasher.update(&fr_to_bytes(&public_inputs.msg_hash));
        for pk in &public_inputs.pks {
            hasher.update(&fr_to_bytes(pk));
        }
        let mut sig = [0u8; 128];
        hasher.finalize_variable(&mut sig).unwrap();

        Self(sig)
    }

    #[must_use]
    pub const fn from_bytes(sig: [u8; 128]) -> Self {
        Self(sig)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> [u8; 128] {
        self.0
    }
}

pub trait ZkSignatureProof {
    /// Verify the proof against the public inputs.
    fn verify(&self, public_inputs: &ZkSignaturePublic) -> bool;
}

impl ZkSignatureProof for DummyZkSignature {
    fn verify(&self, public_inputs: &ZkSignaturePublic) -> bool {
        &Self::prove(public_inputs) == self
    }
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZkSignaturePublic {
    #[serde(with = "serde_fr")]
    pub msg_hash: Fr,
    #[serde_as(as = "Vec<FrDef>")]
    pub pks: Vec<Fr>,
}

struct FrDef;

impl SerializeAs<Fr> for FrDef {
    fn serialize_as<S>(value: &Fr, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde_fr::serialize(value, serializer)
    }
}

impl<'de> DeserializeAs<'de, Fr> for FrDef {
    fn deserialize_as<D>(deserializer: D) -> Result<Fr, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        serde_fr::deserialize(deserializer)
    }
}
