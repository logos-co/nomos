use core::fmt::{self, Debug, Formatter};

use groth16::Fr;
use serde::Deserialize;
use zeroize::ZeroizeOnDrop;
use zksign::{PublicKey, SecretKey, Signature};

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

#[derive(Deserialize, ZeroizeOnDrop, Clone)]
#[cfg_attr(feature = "unsafe", derive(serde::Serialize))]
pub struct ZkKey(SecretKey);

impl ZkKey {
    #[must_use]
    pub const fn new(secret_key: SecretKey) -> Self {
        Self(secret_key)
    }

    #[must_use]
    pub(crate) const fn as_fr(&self) -> &Fr {
        self.0.as_fr()
    }
}

impl Debug for ZkKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "ZkKey(<redacted>)")
    }
}

impl PartialEq for ZkKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_fr() == other.0.as_fr()
    }
}

impl Eq for ZkKey {}

#[async_trait::async_trait]
impl SecuredKey for ZkKey {
    type Payload = Fr;
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
