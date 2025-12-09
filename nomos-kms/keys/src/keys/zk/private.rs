use std::sync::LazyLock;

use groth16::{Fr, fr_from_bytes_unchecked};
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;
use zksign::ZkSignError;

use crate::keys::zk::signature::Signature;

static NOMOS_KDF: LazyLock<Fr> = LazyLock::new(|| fr_from_bytes_unchecked(b"NOMOS_KDF"));

/// A ZK secret key exposing methods to retrieve its inner secret value.
///
/// To be used in contexts where a KMS-like key is required, but it's not
/// possible to go through the KMS roundtrip of executing operators.
#[derive(ZeroizeOnDrop, Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnsecuredZkKey(#[serde(with = "groth16::serde::serde_fr")] Fr);

impl UnsecuredZkKey {
    #[must_use]
    pub const fn zero() -> Self {
        Self(Fr::ZERO)
    }

    #[must_use]
    pub const fn one() -> Self {
        Self(Fr::ONE)
    }

    #[must_use]
    pub const fn new(key: Fr) -> Self {
        Self(key)
    }

    #[must_use]
    pub const fn as_fr(&self) -> &Fr {
        &self.0
    }

    #[must_use]
    pub fn into_inner(self) -> Fr {
        self.0
    }

    #[must_use]
    pub fn to_public_key(&self) -> PublicKey {
        PublicKey(<Poseidon2Bn254Hasher as Digest>::compress(&[
            *NOMOS_KDF, self.0,
        ]))
    }

    pub fn sign(&self, data: &Fr) -> Result<Signature, ZkSignError> {
        Self::multi_sign(std::slice::from_ref(self), data)
    }

    pub fn multi_sign(keys: &[Self], data: &Fr) -> Result<Signature, ZkSignError> {
        let sk_inputs = ZkSignPrivateKeysData::try_from(keys)?;
        let inputs = ZkSignWitnessInputs::from_witness_data_and_message_hash(sk_inputs, *data);

        let (signature, _) = prove(&inputs).expect("Signature should succeed");
        Ok(Signature(signature))
    }
}

impl PartialEq for UnsecuredZkKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_fr().0.0.ct_eq(&other.0.as_fr().0.0).into()
    }
}

impl Eq for UnsecuredZkKey {}

impl From<SecretKey> for UnsecuredZkKey {
    fn from(value: SecretKey) -> Self {
        Self(value)
    }
}

impl From<UnsecuredZkKey> for SecretKey {
    fn from(value: UnsecuredZkKey) -> Self {
        value.0.clone()
    }
}

/// Return a reference to the inner secret key.
impl AsRef<SecretKey> for UnsecuredZkKey {
    fn as_ref(&self) -> &SecretKey {
        &self.0
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

impl From<SecretKey> for Fr {
    fn from(secret: SecretKey) -> Self {
        secret.0
    }
}
