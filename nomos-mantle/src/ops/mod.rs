pub mod blob;
mod opcode;
pub(crate) mod serde_;

use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Op {
    Blob(blob::BlobOp),
}
