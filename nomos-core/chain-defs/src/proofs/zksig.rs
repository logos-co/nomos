use bytes::BufMut as _;
use groth16::{Fr, fr_to_bytes, serde::serde_fr};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeAs, SerializeAs, serde_as};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DummyZkSignature {
    pub public_inputs: ZkSignaturePublic,
}

impl DummyZkSignature {
    #[must_use]
    pub const fn prove(public_inputs: ZkSignaturePublic) -> Self {
        Self { public_inputs }
    }

    #[must_use]
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut b = bytes::BytesMut::new();
        b.put_slice(&fr_to_bytes(&self.public_inputs.msg_hash));
        b.put_u8(self.public_inputs.pks.len() as u8);
        for pk in &self.public_inputs.pks {
            b.put_slice(&fr_to_bytes(pk));
        }
        b.freeze().to_vec()
    }
}

pub trait ZkSignatureProof {
    /// Verify the proof against the public inputs.
    fn verify(&self, public_inputs: &ZkSignaturePublic) -> bool;
}

impl ZkSignatureProof for DummyZkSignature {
    fn verify(&self, public_inputs: &ZkSignaturePublic) -> bool {
        public_inputs == &self.public_inputs
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
