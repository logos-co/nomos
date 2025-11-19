use std::fmt::Debug;

use nomos_blend_message::crypto::proofs::{
    quota,
    quota::{
        ProofOfQuota,
        inputs::prove::{PrivateInputs, PublicInputs, private::ProofOfCoreQuotaInputs},
    },
};
use poq::CorePathAndSelectors;
use poseidon2::ZkHash;
use tokio::sync::oneshot;
use tracing::error;

use crate::keys::{
    ZkKey,
    secured_key::{SecureKeyOperator, SecuredKey},
};

pub struct PoQOperator {
    core_path_and_selectors: CorePathAndSelectors,
    public_inputs: PublicInputs,
    key_index: u64,
    response_channel: Option<oneshot::Sender<Result<(ProofOfQuota, ZkHash), quota::Error>>>,
}

impl Debug for PoQOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PoQOperator")
    }
}

impl PoQOperator {
    pub fn new(
        core_path_and_selectors: CorePathAndSelectors,
        public_inputs: PublicInputs,
        key_index: u64,
        response_channel: oneshot::Sender<Result<(ProofOfQuota, ZkHash), quota::Error>>,
    ) -> Self {
        Self {
            core_path_and_selectors,
            public_inputs,
            key_index,
            response_channel: Some(response_channel),
        }
    }
}

#[async_trait::async_trait]
impl SecureKeyOperator for PoQOperator {
    type Key = ZkKey;
    type Error = <ZkKey as SecuredKey>::Error;

    async fn execute(&mut self, key: &Self::Key) -> Result<(), Self::Error> {
        let poq_result = ProofOfQuota::new(
            &self.public_inputs,
            PrivateInputs::new_proof_of_core_quota_inputs(
                self.key_index,
                ProofOfCoreQuotaInputs {
                    core_path_and_selectors: self.core_path_and_selectors,
                    core_sk: *key.as_fr(),
                },
            ),
        );
        if let Err(e) = self
            .response_channel
            .take()
            .expect("Channel to be available")
            .send(poq_result)
        {
            error!("Error building proof of quota: {e:?}");
        }
        Ok(())
    }
}
