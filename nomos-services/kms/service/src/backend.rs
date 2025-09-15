use crate::{
    encodings::{Encoding, EncodingFormat},
    keys::{Ed25519Key, Key, KeyError, ZkKey, secured_key::SecuredKey},
};

enum KmsBackendError {
    KeyError(KeyError),
    KeyNotFound,
}

impl From<KeyError> for KmsBackendError {
    fn from(error: KeyError) -> Self {
        KmsBackendError::KeyError(error)
    }
}

pub trait KmsBackend {
    type KeyId;
    type SupportedKey: SecuredKey;
    type Error;

    fn sign(
        &self,
        key_id: Self::KeyId,
        data: <Self::SupportedKey as SecuredKey>::Encoding,
    ) -> Result<<Self::SupportedKey as SecuredKey>::Encoding, Self::Error>;
}

////////// Ed25519Backend //////////

struct Ed25519Backend;

impl Ed25519Backend {
    fn get_key(
        &self,
        key_id: &String,
    ) -> Result<<Self as KmsBackend>::SupportedKey, <Self as KmsBackend>::Error> {
        unimplemented!()
    }
}

impl KmsBackend for Ed25519Backend {
    type KeyId = String;
    type SupportedKey = Ed25519Key;
    type Error = KmsBackendError;

    fn sign(
        &self,
        key_id: Self::KeyId,
        data: <Self::SupportedKey as SecuredKey>::Encoding,
    ) -> Result<<Self::SupportedKey as SecuredKey>::Encoding, Self::Error> {
        let key = self.get_key(&key_id)?;
        let signature = key.sign(data);
        signature
            .map(<Self::SupportedKey as SecuredKey>::Encoding::from)
            .map_err(Self::Error::from)
    }
}

//////////////////////////////////////////////////

////////// KeyBackend //////////

struct KeyBackend;

impl KeyBackend {
    fn get_key(
        &self,
        key_id: &String,
    ) -> Result<<Self as KmsBackend>::SupportedKey, <Self as KmsBackend>::Error> {
        unimplemented!()
    }
}

impl KmsBackend for KeyBackend {
    type KeyId = String;
    type SupportedKey = Key;
    type Error = KmsBackendError;

    fn sign(
        &self,
        key_id: Self::KeyId,
        data: <Self::SupportedKey as SecuredKey>::Encoding,
    ) -> Result<<Self::SupportedKey as SecuredKey>::Encoding, Self::Error> {
        let key = self.get_key(&key_id)?;
        let signature = key.sign(data);
        signature
            .map(<Self::SupportedKey as SecuredKey>::Encoding::from)
            .map_err(Self::Error::from)
    }
}

//////////////////////////////////////////////////

////////// SubsetBackend //////////

pub enum SupersetEncoding {
    // No need, since it's implicit in Key.
    Encoding(EncodingFormat),
    Other(String),
}

impl Encoding for SupersetEncoding {}

struct SubsetBackend;

impl SubsetBackend {
    fn get_key(
        &self,
        key_id: &String,
    ) -> Result<<Self as KmsBackend>::SupportedKey, <Self as KmsBackend>::Error> {
        unimplemented!()
    }
}

impl KmsBackend for SubsetBackend {
    type KeyId = String;
    type SupportedKey = Key;
    type Error = KmsBackendError;

    fn sign(
        &self,
        key_id: Self::KeyId,
        data: <Self::SupportedKey as SecuredKey>::Encoding,
    ) -> Result<<Self::SupportedKey as SecuredKey>::Encoding, Self::Error> {
        // Same use case as KeyBackend.
        unimplemented!()
    }
}

//////////////////////////////////////////////////

////////// SupersetBackend //////////

pub struct NewKey;

impl SecuredKey for NewKey {
    type Encoding = SupersetEncoding;

    fn sign(&self, data: Self::Encoding) -> Result<Self::Encoding, KeyError> {
        unimplemented!()
    }
}

// Set of keys whose encoding doesn't cover all its keys' encodings.
// TODO: Is there a way to force variants to have the same Encoding? Is it
// necessary?
pub enum SupersetKey {
    Key(Key),       // Encoding
    NewKey(NewKey), // SupersetEncoding: Encoding + Other
}

impl SecuredKey for SupersetKey {
    type Encoding = EncodingFormat;

    fn sign(&self, data: Self::Encoding) -> Result<Self::Encoding, KeyError> {
        match self {
            SupersetKey::Key(key) => key.sign(data),
            SupersetKey::NewKey(new_key) => {
                let superset_data = SupersetEncoding::Encoding(data);
                let signature = new_key.sign(superset_data);
                match signature {
                    Ok(SupersetEncoding::Encoding(signature)) => Ok(signature),
                    Ok(_) => Err(KeyError::Any(String::from(
                        "Encoding not covered by NewKey.",
                    ))),
                    Err(error) => Err(error),
                }
            }
        }
    }
}

struct SupersetBackend;

impl SupersetBackend {
    fn get_key(
        &self,
        key_id: &String,
    ) -> Result<<Self as KmsBackend>::SupportedKey, <Self as KmsBackend>::Error> {
        unimplemented!()
    }
}

impl KmsBackend for SupersetBackend {
    type KeyId = String;
    type SupportedKey = SupersetKey;
    type Error = KmsBackendError;

    fn sign(
        &self,
        key_id: Self::KeyId,
        data: <Self::SupportedKey as SecuredKey>::Encoding,
    ) -> Result<<Self::SupportedKey as SecuredKey>::Encoding, Self::Error> {
        let key = self.get_key(&key_id)?;
        let signature = key.sign(data);
        signature
            .map(<Self::SupportedKey as SecuredKey>::Encoding::from)
            .map_err(Self::Error::from)
    }
}

//////////////////////////////////////////////////

////////// NestedBackend //////////

pub enum NestedKey {
    Key(Key),
    Other(String),
}

impl SecuredKey for NestedKey {
    type Encoding = EncodingFormat;

    fn sign(&self, data: Self::Encoding) -> Result<Self::Encoding, KeyError> {
        match self {
            NestedKey::Key(key) => key.sign(data),
            NestedKey::Other(_) => Err(KeyError::Any(String::from("Not implemented yet."))),
        }
    }
}

struct NestedBackend;

impl NestedBackend {
    fn get_key(
        &self,
        key_id: &String,
    ) -> Result<<Self as KmsBackend>::SupportedKey, <Self as KmsBackend>::Error> {
        unimplemented!()
    }
}

impl KmsBackend for NestedBackend {
    type KeyId = String;
    type SupportedKey = NestedKey;
    type Error = KmsBackendError;

    fn sign(
        &self,
        key_id: Self::KeyId,
        data: <Self::SupportedKey as SecuredKey>::Encoding,
    ) -> Result<<Self::SupportedKey as SecuredKey>::Encoding, Self::Error> {
        let key = self.get_key(&key_id)?;
        let signature = key.sign(data);
        signature
            .map(<Self::SupportedKey as SecuredKey>::Encoding::from)
            .map_err(Self::Error::from)
    }
}
