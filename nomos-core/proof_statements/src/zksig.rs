use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZkSignaturePublic {
    pub msg_hash: [u8; 32],
    pub pks: Vec<[u8; 32]>,
}
