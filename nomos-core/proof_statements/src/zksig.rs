use groth16::{serde::serde_fr, Fr};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZkSignaturePublic {
    #[serde(with = "serde_fr")]
    pub msg_hash: Fr,
    // TODO: implement serde for this
    #[serde(skip)]
    pub pks: Vec<Fr>,
}
