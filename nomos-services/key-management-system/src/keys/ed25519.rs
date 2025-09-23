use bytes::Bytes as RawBytes;
use ed25519_dalek::ed25519::signature::Signer as _;
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

use crate::{
    encodings::Encoding,
    keys::{KeyError, secured_key::SecuredKey},
};

type InnerEncoding = crate::encodings::Bytes;

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct Ed25519Key(pub(crate) ed25519_dalek::SigningKey);

impl SecuredKey<InnerEncoding> for Ed25519Key {
    type Signature = InnerEncoding;
    type PublicKey = InnerEncoding;
    type Error = KeyError;

    fn sign(&self, data: &InnerEncoding) -> Result<Self::Signature, Self::Error> {
        let data_bytes = data.as_bytes();
        let signature_slice = self.0.sign(data_bytes).to_bytes();
        let signature_bytes = RawBytes::copy_from_slice(&signature_slice);
        Ok(Self::Signature::from(signature_bytes))
    }

    fn as_public_key(&self) -> Self::PublicKey {
        let verifying_key = self.0.verifying_key();
        let verifying_key_bytes = RawBytes::copy_from_slice(verifying_key.as_bytes());
        Self::PublicKey::from(verifying_key_bytes)
    }
}

impl SecuredKey<Encoding> for Ed25519Key {
    type Signature = Encoding;
    type PublicKey = Encoding;
    type Error = KeyError;

    fn sign(&self, data: &Encoding) -> Result<Self::Signature, Self::Error> {
        match data {
            Encoding::Bytes(data) => self.sign(data).map(Encoding::from),
        }
    }

    fn as_public_key(&self) -> Self::PublicKey {
        <Self as SecuredKey<InnerEncoding>>::as_public_key(self).into()
    }
}
