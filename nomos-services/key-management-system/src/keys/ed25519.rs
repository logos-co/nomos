use bytes::Bytes;
use ed25519_dalek::ed25519::signature::Signer as _;
use overwatch::DynError;
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::secure_key::SecuredKey;

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct Ed25519Key(pub(crate) ed25519_dalek::SigningKey);

impl SecuredKey for Ed25519Key {
    fn sign(&self, data: Bytes) -> Result<Bytes, DynError> {
        Ok(Bytes::copy_from_slice(&self.0.sign(&data).to_bytes()))
    }

    fn as_pk(&self) -> Bytes {
        Bytes::copy_from_slice(self.0.verifying_key().as_bytes())
    }
}
