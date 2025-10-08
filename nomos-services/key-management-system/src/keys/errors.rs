use std::any::{type_name, type_name_of_val};

use thiserror::Error;

use crate::keys::secured_key::SecuredKey;

#[allow(
    clippy::allow_attributes,
    reason = "Below's `dead_code` will not trigger when keys are enabled. This reduces a warning when running `cargo hack`."
)]
#[allow(
    dead_code,
    reason = "Variants' usage depends on feature gates: At any point in time, at least one will be unused."
)]
#[derive(Error, Debug)]
pub enum KeyError {
    #[error(transparent)]
    Encoding(EncodingError),
    #[error("Unsupported multikey: {0}")]
    UnsupportedKey(String),
    #[error("Multisignature support only {0} keys, got {1}")]
    UnsupportedMultisignatureSize(usize, usize),
}

impl From<EncodingError> for KeyError {
    fn from(value: EncodingError) -> Self {
        Self::Encoding(value)
    }
}

#[derive(Error, Debug)]
pub enum EncodingError {
    #[error("Required encoding: {0}")]
    Requires(String),
}

impl EncodingError {
    /// Creates a new `EncodingError::Requires` error.
    #[expect(dead_code, reason = "Will be used when integrating KMS.")]
    pub fn requires<Key: SecuredKey, Payload>(key: &Key, received_payload: &Payload) -> Self {
        let key_type_name = type_name_of_val(key);
        let payload_type_name = type_name::<Key::Payload>().to_owned();
        let received_payload_type_name = type_name_of_val(received_payload);
        Self::Requires(format!(
            "Key of type `{key_type_name}` requires a payload of type `{payload_type_name}`, but got payload of type `{received_payload_type_name}`",
        ))
    }
}
