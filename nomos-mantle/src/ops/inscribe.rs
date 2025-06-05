use serde::{Deserialize, Serialize};

use crate::ops::TxHash;

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone)]
pub struct InscriptionOp {
    /// Message to be written in the blockchain
    inscription: Vec<u8>,
    /// Enforce that this inscription comes after this tx
    after_tx: Option<TxHash>,
}
