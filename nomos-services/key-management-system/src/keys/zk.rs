use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;
use zksign::{PublicKey, SecretKey, Signature};

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug, ZeroizeOnDrop)]
pub struct ZkKey(pub SecretKey);

impl SecuredKey for ZkKey {
    type Payload = groth16::Fr;
    type Signature = Signature;
    type PublicKey = PublicKey;
    type Error = KeyError;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
        Ok(self.0.sign(payload)?)
    }

    fn sign_multiple(
        keys: &[&Self],
        payload: &Self::Payload,
    ) -> Result<Self::Signature, Self::Error> {
        Ok(SecretKey::multi_sign(
            &keys.iter().map(|key| key.0.clone()).collect::<Vec<_>>(),
            payload,
        )?)
    }

    fn as_public_key(&self) -> Self::PublicKey {
        self.0.to_public_key()
    }
}
