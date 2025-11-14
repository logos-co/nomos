use futures::future::ready;
use nomos_blend_message::crypto::{
    keys::Ed25519PublicKey,
    proofs::{
        PoQVerificationInputsMinusSigningKey,
        quota::{
            self, ProofOfQuota,
            fixtures::{valid_proof_of_core_quota_inputs, valid_proof_of_leadership_quota_inputs},
            inputs::prove::{
                PrivateInputs, PublicInputs as PoQPublicInputs,
                private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
            },
        },
    },
};
use nomos_core::crypto::ZkHash;

use crate::message_blend::CoreProofOfQuotaGenerator;

pub const fn poq_public_inputs_from_session_public_inputs_and_signing_key(
    (
        PoQVerificationInputsMinusSigningKey {
            core,
            leader,
            session,
        },
        signing_key,
    ): (PoQVerificationInputsMinusSigningKey, Ed25519PublicKey),
) -> PoQPublicInputs {
    PoQPublicInputs {
        signing_key,
        core,
        leader,
        session,
    }
}

pub fn valid_proof_of_quota_inputs(
    core_quota: u64,
) -> (PoQVerificationInputsMinusSigningKey, ProofOfCoreQuotaInputs) {
    let (
        PoQPublicInputs {
            core,
            leader,
            session,
            ..
        },
        private_inputs,
    ) = valid_proof_of_core_quota_inputs([0; _].try_into().unwrap(), core_quota);
    (
        PoQVerificationInputsMinusSigningKey {
            core,
            leader,
            session,
        },
        private_inputs,
    )
}

pub fn valid_proof_of_leader_inputs(
    leader_quota: u64,
) -> (
    PoQVerificationInputsMinusSigningKey,
    ProofOfLeadershipQuotaInputs,
) {
    let (
        PoQPublicInputs {
            core,
            leader,
            session,
            ..
        },
        private_inputs,
    ) = valid_proof_of_leadership_quota_inputs([0; _].try_into().unwrap(), leader_quota);
    (
        PoQVerificationInputsMinusSigningKey {
            core,
            leader,
            session,
        },
        private_inputs,
    )
}

#[derive(Clone)]
pub struct CorePoQGeneratorFromPrivateCoreQuotaInputs(ProofOfCoreQuotaInputs);

impl CorePoQGeneratorFromPrivateCoreQuotaInputs {
    pub fn new(private_inputs: ProofOfCoreQuotaInputs) -> Self {
        Self(private_inputs)
    }
}

impl CoreProofOfQuotaGenerator for CorePoQGeneratorFromPrivateCoreQuotaInputs {
    fn generate_poq(
        &self,
        public_inputs: &PoQPublicInputs,
        key_index: u64,
    ) -> impl Future<Output = Result<(ProofOfQuota, ZkHash), quota::Error>> + Send + Sync {
        ready(ProofOfQuota::new(
            public_inputs,
            PrivateInputs::new_proof_of_core_quota_inputs(key_index, self.0.clone()),
        ))
    }
}
