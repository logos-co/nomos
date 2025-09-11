use groth16::{serde::serde_fr, Fr};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DeserializeAs, SerializeAs};

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
