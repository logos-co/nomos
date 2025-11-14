use groth16::Fr;
use nomos_blend_message::crypto::proofs::quota::{
    ProofOfQuota,
    inputs::prove::{PrivateInputs, PublicInputs, private::ProofOfCoreQuotaInputs},
};
use poq::CorePathAndSelectors;
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;
use zksign::{PublicKey, SecretKey, Signature};

use crate::keys::{errors::KeyError, secured_key::SecuredKey};

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug, ZeroizeOnDrop)]
pub struct ZkKey(SecretKey);

impl ZkKey {
    #[must_use]
    pub const fn new(secret_key: SecretKey) -> Self {
        Self(secret_key)
    }
}

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

    fn generate_core_poq(
        &self,
        public_inputs: &PublicInputs,
        key_index: u64,
        core_path_and_selectors: CorePathAndSelectors,
    ) -> Result<(ProofOfQuota, Fr), Self::Error> {
        ProofOfQuota::new(
            public_inputs,
            PrivateInputs::new_proof_of_core_quota_inputs(
                key_index,
                ProofOfCoreQuotaInputs {
                    core_path_and_selectors,
                    core_sk: *self.0.as_fr(),
                },
            ),
        )
        .map_err(KeyError::PoQGeneration)
    }
}
