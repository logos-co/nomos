use serde::{Deserialize, Serialize};

use crate::mantle::{
    gas::{Gas, GasConstants, GasPrice},
    tx::TxHash,
};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct InscriptionOp {
    /// Message to be written in the blockchain
    inscription: Vec<u8>,
    /// Enforce that this inscription comes after this tx
    after_tx: Option<TxHash>,
}

impl GasPrice for InscriptionOp {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        let ordered = u64::from(self.after_tx.is_some());
        Constants::INSCRIBE_BASE_GAS
            + Constants::INSCRIBE_BYTE_GAS * self.inscription.len() as u64
            + Constants::INSCRIBE_ORDERING_GAS * ordered
    }
}
