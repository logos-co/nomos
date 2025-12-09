use core::fmt::{self, Debug, Formatter};
use std::sync::LazyLock;

use groth16::{Field as _, Fr, fr_from_bytes_unchecked};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;
use zeroize::ZeroizeOnDrop;

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

mod private;
mod public;
mod signature;

/// An hardened ZK secret key that only exposes methods to retrieve public
/// information.
///
/// It is a secured variant of a [`UnsecuredZkKey`] and used within the set of
/// supported KMS keys.
#[derive(Deserialize, ZeroizeOnDrop, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "unsafe", derive(serde::Serialize))]
pub struct ZkKey(UnsecuredZkKey);

impl ZkKey {
    #[must_use]
    pub const fn new(secret_key: SecretKey) -> Self {
        Self(UnsecuredZkKey(secret_key))
    }
}

impl Debug for ZkKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let private_key = if cfg!(feature = "unsafe") {
            format!("{:?}", self.0)
        } else {
            "<redacted>".to_owned()
        };
        write!(f, "ZkKey({private_key})")
    }
}

#[cfg(feature = "unsafe")]
impl From<UnsecuredZkKey> for ZkKey {
    fn from(value: UnsecuredZkKey) -> Self {
        Self(value)
    }
}

impl From<SecretKey> for ZkKey {
    fn from(value: SecretKey) -> Self {
        Self(UnsecuredZkKey::from(value))
    }
}

#[async_trait::async_trait]
impl SecuredKey for ZkKey {
    type Payload = Fr;
    type Signature = Signature;
    type PublicKey = PublicKey;
    type Error = KeyError;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
        Ok(self.0.as_ref().sign(payload)?)
    }

    fn sign_multiple(
        keys: &[&Self],
        payload: &Self::Payload,
    ) -> Result<Self::Signature, Self::Error> {
        Ok(SecretKey::multi_sign(
            &keys
                .iter()
                .map(|key| key.0.as_ref().clone())
                .collect::<Vec<_>>(),
            payload,
        )?)
    }

    fn as_public_key(&self) -> Self::PublicKey {
        self.0.as_ref().to_public_key()
    }
}
