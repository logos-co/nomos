use bytes::Bytes as RawBytes;
use ed25519_dalek::ed25519::signature::Signer as _;
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::keys::{KeyError, secured_key::SecuredKey};

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct Ed25519Key(pub(crate) ed25519_dalek::SigningKey);

impl SecuredKey for Ed25519Key {
    type Encoding = crate::encodings::Bytes;
    type Error = KeyError;

    fn sign(&self, data: Self::Encoding) -> Result<Self::Encoding, Self::Error> {
        let data_bytes = data.as_bytes();
        let signature_slice = self.0.sign(data_bytes).to_bytes();
        let signature_bytes = RawBytes::copy_from_slice(&signature_slice);
        Ok(Self::Encoding::from(signature_bytes))
    }

    fn as_pk(&self) -> Self::Encoding {
        let verifying_key = self.0.verifying_key();
        let verifying_key_bytes = RawBytes::copy_from_slice(verifying_key.as_bytes());
        Self::Encoding::from(verifying_key_bytes)
    }
}
