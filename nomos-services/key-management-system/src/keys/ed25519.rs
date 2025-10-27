use bytes::Bytes;
use ed25519_dalek::{Signature, VerifyingKey, ed25519::signature::Signer as _};
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug, ZeroizeOnDrop)]
pub struct Ed25519Key(pub(crate) ed25519_dalek::SigningKey);

impl SecuredKey for Ed25519Key {
    type Payload = Bytes;
    type Signature = Signature;
    type PublicKey = VerifyingKey;
    type Error = KeyError;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
        Ok(self.0.sign(payload.iter().as_slice()))
    }

    fn sign_multiple(
        _keys: &[&Self],
        _payload: &Self::Payload,
    ) -> Result<Self::Signature, Self::Error> {
        unimplemented!("Multi-key signature is not implemented for Ed25519 keys.")
    }

    fn as_public_key(&self) -> VerifyingKey {
        self.0.verifying_key()
    }
}
