use std::sync::LazyLock;

use ark_ff::Field as _;
use generic_array::{
    GenericArray,
    typenum::{U32, U64},
};
use groth16::{Fr, serde::serde_fr};
use num_bigint::BigUint;
use poseidon2::{Digest as _, Poseidon2Bn254Hasher};
use serde::{Deserialize, Serialize};
use tracing::error;
use zeroize::ZeroizeOnDrop;
use zksign::{ZkSignProof, ZkSignVerifierInputs, ZkSignWitnessInputs, prove, verify};
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(transparent)]
pub struct SecretKey(#[serde(with = "serde_fr")] Fr);

static NOMOS_KDF_V1: LazyLock<Fr> =
    LazyLock::new(|| BigUint::from_bytes_le(b"NOMOS_KDF_V1").into());

impl SecretKey {
    #[must_use]
    pub const fn zero() -> Self {
        Self(Fr::ZERO)
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
    pub fn to_public_key(&self) -> PublicKey {
        PublicKey(Poseidon2Bn254Hasher::digest(&[*NOMOS_KDF_V1, self.0]))
    }

    #[must_use]
    pub fn sign(&self, data: &Fr) -> Signature {
        let mut keys = [const { Self::zero() }; 32];
        keys[0] = self.clone();
        Self::multi_sign(&keys, data)
    }

    #[must_use]
    pub fn multi_sign(keys: &[Self; 32], data: &Fr) -> Signature {
        let keys: [_; 32] = keys
            .iter()
            .map(|k| k.0)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Size is correct from method signature");
        let inputs = ZkSignWitnessInputs::from_witness_data_and_message_hash(keys.into(), *data);
        let (signature, _) = prove(&inputs).expect("Signature should succeed");
        Signature(signature)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicKey(#[serde(with = "serde_fr")] Fr);

impl PublicKey {
    #[must_use]
    pub const fn zero() -> Self {
        Self(Fr::ZERO)
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
    pub fn verify(&self, data: &Fr, signature: &Signature) -> bool {
        let mut pks = [const { Self::zero() }; 32];
        pks[0] = *self;
        Self::verify_multi(&pks, data, signature)
    }

    #[must_use]
    pub fn verify_multi(pks: &[Self; 32], data: &Fr, signature: &Signature) -> bool {
        let pks = pks
            .iter()
            .map(|pk| pk.0)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Size is correct from method signature");
        let inputs = ZkSignVerifierInputs::new_from_msg_and_pks(*data, &pks);
        verify(signature.as_proof(), &inputs).unwrap_or_else(|e| {
            error!("Error verifying signature: {e:?}");
            false
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ZkSignProof")]
struct SignatureSerde {
    pi_a: GenericArray<u8, U32>,
    pi_b: GenericArray<u8, U64>,
    pi_c: GenericArray<u8, U32>,
}

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct Signature(#[serde(with = "SignatureSerde")] ZkSignProof);

impl Signature {
    #[must_use]
    pub const fn as_proof(&self) -> &ZkSignProof {
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
