use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct NativeOp;

impl NativeOp {
    #[must_use]
    pub const fn payload_bytes(&self) -> Bytes {
        Bytes::new()
    }
}
