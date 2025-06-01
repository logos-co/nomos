use serde::{Deserialize, Serialize};

use crate::gas::{Gas, GasConstants, GasPrice};

#[derive(Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct NativeOp;

impl GasPrice for NativeOp {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        // penalizing inclusion of native operations for now
        u64::MAX
    }
}
