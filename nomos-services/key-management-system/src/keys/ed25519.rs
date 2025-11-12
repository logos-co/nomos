use bytes::Bytes;
use ed25519_dalek::{Signature, VerifyingKey, ed25519::signature::Signer as _};
use groth16::Fr;
use nomos_blend_message::crypto::proofs::quota::{ProofOfQuota, inputs::prove::PublicInputs};
use nomos_utils::serde::{deserialize_bytes_array, serialize_bytes_array};
use poq::CorePathAndSelectors;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::ZeroizeOnDrop;

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

#[derive(PartialEq, Eq, Clone, Debug, ZeroizeOnDrop)]
pub struct Ed25519Key(ed25519_dalek::SigningKey);

impl Ed25519Key {
    #[must_use]
    pub const fn new(signing_key: ed25519_dalek::SigningKey) -> Self {
        Self(signing_key)
    }
}

impl Serialize for Ed25519Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_bytes_array(self.0.to_bytes(), serializer)
    }
}

impl<'de> Deserialize<'de> for Ed25519Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = deserialize_bytes_array(deserializer)?;
        Ok(Self(ed25519_dalek::SigningKey::from_bytes(&bytes)))
    }
}

impl SecuredKey for Ed25519Key {
    type Payload = Bytes;
    type Signature = Signature;
    type PublicKey = VerifyingKey;
    type Error = KeyError;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
        Ok(self.0.sign(payload.iter().as_slice()))
    }

    fn sign_multiple(
        _keys: &[&Self],
        _payload: &Self::Payload,
    ) -> Result<Self::Signature, Self::Error> {
        unimplemented!("Multi-key signature is not implemented for Ed25519 keys.")
    }

    fn as_public_key(&self) -> VerifyingKey {
        self.0.verifying_key()
    }

    fn generate_core_poq(
        &self,
        _public_inputs: &PublicInputs,
        _key_index: u64,
        _core_path_and_selectors: CorePathAndSelectors,
    ) -> Result<(ProofOfQuota, Fr), Self::Error> {
        unimplemented!("Core PoQ generation is not implemented for Ed25519 keys.")
    }
}
