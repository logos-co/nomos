use serde::{Deserialize, Serialize};

use crate::mantle::gas::{Gas, GasConstants, GasCost};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct NativeOp;
