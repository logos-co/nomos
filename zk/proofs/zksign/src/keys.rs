use std::sync::LazyLock;

use ark_ff::Field as _;
use generic_array::{
    GenericArray,
    typenum::{U32, U64},
};
use groth16::{Fr, fr_from_bytes, serde::serde_fr};
use num_bigint::BigUint;
use poseidon2::{Digest, Poseidon2Bn254Hasher};
use serde::{Deserialize, Serialize};
use tracing::error;
use zeroize::ZeroizeOnDrop;

use crate::{ZkSignPrivateKeysData, ZkSignProof, ZkSignVerifierInputs, ZkSignWitnessInputs, prove};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(transparent)]
pub struct SecretKey(#[serde(with = "serde_fr")] Fr);

static NOMOS_KDF: LazyLock<Fr> =
    LazyLock::new(|| fr_from_bytes(b"NOMOS_KDF").expect("constant should be valid"));

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
    pub fn into_inner(self) -> Fr {
        self.0
    }

    #[must_use]
    pub fn to_public_key(&self) -> PublicKey {
        PublicKey(<Poseidon2Bn254Hasher as Digest>::compress(&[
            *NOMOS_KDF, self.0,
        ]))
    }

    pub fn sign(&self, data: &Fr) -> Result<Signature, crate::ZkSignError> {
        Self::multi_sign(std::slice::from_ref(self), data)
    }

    pub fn multi_sign(keys: &[Self], data: &Fr) -> Result<Signature, crate::ZkSignError> {
        let sk_inputs = ZkSignPrivateKeysData::try_from(keys)?;
        let inputs = ZkSignWitnessInputs::from_witness_data_and_message_hash(sk_inputs, *data);

        let (signature, _) = prove(&inputs).expect("Signature should succeed");
        Ok(Signature(signature))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
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
    pub const fn into_inner(self) -> Fr {
        self.0
    }

    #[must_use]
    pub fn verify(&self, data: &Fr, signature: &Signature) -> bool {
        let mut pks = [const { Self::zero() }; 32];
        pks[0] = *self;
        Self::verify_multi(&pks, data, signature)
    }

    #[must_use]
    pub fn verify_multi(pks: &[Self], data: &Fr, signature: &Signature) -> bool {
        let inputs = match ZkSignVerifierInputs::try_from_pks((*data).into(), pks) {
            Ok(inputs) => inputs,
            Err(e) => {
                error!("Error building verifier inputs: {e:?}");
                return false;
            }
        };

        crate::verify(signature.as_proof(), &inputs).unwrap_or_else(|e| {
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

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Signature(#[serde(with = "SignatureSerde")] ZkSignProof);

impl Signature {
    #[must_use]
    pub const fn new(proof: ZkSignProof) -> Self {
        Self(proof)
    }

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
