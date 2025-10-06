use nomos_core::mantle::keys::{PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct ZkKey(pub(crate) SecretKey);

impl SecuredKey for ZkKey {
    type Payload = groth16::Fr;
    type Signature = Signature;
    type PublicKey = PublicKey;
    type Error = KeyError;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
        Ok(self.0.sign(payload))
    }

    fn sign_multiple(
        keys: &[&Self],
        payload: &Self::Payload,
    ) -> Result<Self::Signature, Self::Error> {
        if keys.len() != 32 {
            return Err(KeyError::UnsupportedMultisignatureSize(32, keys.len()));
        }
        let keys: [_; 32] = keys
            .iter()
            .map(|k| k.0.clone())
            .collect::<Vec<_>>()
            .try_into()
            .expect("Error is checked first thing in the method");
        Ok(SecretKey::multi_sign(&keys, payload))
    }

    fn as_public_key(&self) -> Self::PublicKey {
        self.0.to_public_key()
    }
}
