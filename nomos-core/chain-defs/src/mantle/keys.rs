use std::sync::LazyLock;

use groth16::{Fr, serde::serde_fr};
use num_bigint::BigUint;
use poseidon2::{Digest as _, Poseidon2Bn254Hasher};
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(transparent)]
pub struct SecretKey(#[serde(with = "serde_fr")] Fr);

static NOMOS_KDF_V1: LazyLock<Fr> =
    LazyLock::new(|| BigUint::from_bytes_le(b"NOMOS_KDF_V1").into());

impl SecretKey {
    #[must_use]
    pub const fn new(key: Fr) -> Self {
        Self(key)
    }

    #[must_use]
    pub const fn as_fr(&self) -> &Fr {
        &self.0
    }

    #[must_use]
    pub fn to_public_key(&self) -> PublicKey {
        PublicKey(Poseidon2Bn254Hasher::digest(&[*NOMOS_KDF_V1, self.0]))
    }

    #[must_use]
    pub fn sign(&self, _data: &Fr) -> Fr {
        unimplemented!("Signing is not implemented yet")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicKey(#[serde(with = "serde_fr")] Fr);

impl PublicKey {
    #[must_use]
    pub const fn new(key: Fr) -> Self {
        Self(key)
    }

    #[must_use]
    pub const fn as_fr(&self) -> &Fr {
        &self.0
    }
}

impl From<SecretKey> for PublicKey {
    fn from(secret: SecretKey) -> Self {
        secret.to_public_key()
    }
}

impl From<Fr> for SecretKey {
    fn from(key: Fr) -> Self {
        Self::new(key)
    }
}

impl From<BigUint> for SecretKey {
    fn from(value: BigUint) -> Self {
        Self(value.into())
    }
}

impl From<Fr> for PublicKey {
    fn from(key: Fr) -> Self {
        Self::new(key)
    }
}

impl From<BigUint> for PublicKey {
    fn from(value: BigUint) -> Self {
        Self(value.into())
    }
}

impl From<SecretKey> for Fr {
    fn from(secret: SecretKey) -> Self {
        secret.0
    }
}

impl From<PublicKey> for Fr {
    fn from(public: PublicKey) -> Self {
        public.0
    }
}
