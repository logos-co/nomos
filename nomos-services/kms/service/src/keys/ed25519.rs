use crate::{
    encodings,
    keys::{KeyError, secured_key::SecuredKey},
};

pub struct Ed25519Key;

impl SecuredKey for Ed25519Key {
    type Encoding = encodings::Bytes;

    fn sign(&self, data: Self::Encoding) -> Result<Self::Encoding, KeyError> {
        unimplemented!()
    }
}
