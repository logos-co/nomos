use nomos_core::mantle::keys::{PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct ZkKey(pub(crate) SecretKey);

impl SecuredKey for ZkKey {
    type Payload = groth16::Fr;
    type Signature = groth16::Fr;
    type PublicKey = PublicKey;
    type Error = KeyError;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
        Ok(self.0.sign(payload))
    }

    fn as_public_key(&self) -> Self::PublicKey {
        self.0.to_public_key()
    }
}
