use crate::{
    encodings,
    keys::{KeyError, secured_key::SecuredKey},
};

pub struct ZkKey;

impl SecuredKey for ZkKey {
    type Encoding = encodings::Fr;

    fn sign(&self, data: Self::Encoding) -> Result<Self::Encoding, KeyError> {
        unimplemented!()
    }
}
