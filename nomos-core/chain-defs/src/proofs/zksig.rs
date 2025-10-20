use blake2::Digest as _;
use generic_array::{GenericArray, typenum::U128};
use groth16::{Fr, fr_to_bytes, serde::serde_fr};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeAs, SerializeAs, serde_as};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "GenericArray<u8, U128>", into = "GenericArray<u8, U128>")]
pub struct DummyZkSignature([u8; 128]);

#[expect(clippy::from_over_into, reason = "GenericArray is a foreign type")]
impl Into<GenericArray<u8, U128>> for DummyZkSignature {
    fn into(self) -> GenericArray<u8, U128> {
        GenericArray::from_array(self.0)
    }
}

impl From<GenericArray<u8, U128>> for DummyZkSignature {
    fn from(sig: GenericArray<u8, U128>) -> Self {
        let mut arr = [0u8; 128];
        arr[..].copy_from_slice(&sig);
        Self::from_bytes(arr)
    }
}

impl DummyZkSignature {
    #[must_use]
    pub fn prove(public_inputs: &ZkSignaturePublic) -> Self {
        let mut hasher = blake2::Blake2b512::new();
        hasher.update(fr_to_bytes(&public_inputs.msg_hash));
        for pk in &public_inputs.pks {
            hasher.update(fr_to_bytes(pk));
        }

        // Blake2b supports up to 512bit (64byte) hashes, meaning
        // only first half of the sig will be filled.

        let mut sig = [0u8; 128];
        sig[..64].copy_from_slice(&hasher.finalize());

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
