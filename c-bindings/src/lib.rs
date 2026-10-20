#![allow(
    unsafe_code,
    reason = "The C API is built on raw pointers and `extern \"C\"` exports."
)]

pub mod api;
mod callbacks;
mod errors;
#[cfg(test)]
mod ffi_safety_tests;
pub(crate) mod logging;
mod macros;
mod node;
mod option;
mod result;

pub use errors::{OperationStatus, OperationStatusCode, free_operation_status};
pub use node::LogosBlockchainNode;
pub use result::{FfiResult, FfiStatusResult, StatusResult};
