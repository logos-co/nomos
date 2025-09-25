use bytes::Bytes;
use ed25519_dalek::{Signature, VerifyingKey, ed25519::signature::Signer as _};
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::{
    encodings::{DataEncoding, PublicKeyEncoding, SignatureEncoding},
    keys::{KeyError, secured_key::SecuredKey},
};

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct Ed25519Key(pub(crate) ed25519_dalek::SigningKey);

impl SecuredKey<Bytes> for Ed25519Key {
    type Signature = Signature;
    type PublicKey = VerifyingKey;
    type Error = KeyError;

    fn sign(&self, data: &Bytes) -> Result<Self::Signature, Self::Error> {
        Ok(self.0.sign(data.iter().as_slice()))
    }

    fn as_public_key(&self) -> Self::PublicKey {
        self.0.verifying_key()
    }
}

impl SecuredKey<DataEncoding> for Ed25519Key {
    type Signature = SignatureEncoding;
    type PublicKey = PublicKeyEncoding;
    type Error = KeyError;

    fn sign(&self, data: &DataEncoding) -> Result<Self::Signature, Self::Error> {
        match data {
            DataEncoding::Ed25519(data) => {
                let signature = <Self as SecuredKey<Bytes>>::sign(self, data)?;
                Ok(Self::Signature::Ed25519(signature))
            }
        }
    }

    fn as_public_key(&self) -> Self::PublicKey {
        let public_key = <Self as SecuredKey<Bytes>>::as_public_key(self);
        Self::PublicKey::Ed25519(public_key)
    }
}
