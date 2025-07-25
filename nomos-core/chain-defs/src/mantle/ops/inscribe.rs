use serde::{Deserialize, Serialize};

use crate::mantle::tx::TxHash;

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct InscriptionOp {
    /// Message to be written in the blockchain
    inscription: Vec<u8>,
    /// Enforce that this inscription comes after this tx
    after_tx: Option<TxHash>,
}
