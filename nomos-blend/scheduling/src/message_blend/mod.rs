use async_trait::async_trait;
use nomos_blend_message::crypto::proofs::quota::{self, ProofOfQuota, inputs::prove::PublicInputs};
use nomos_core::crypto::ZkHash;

pub mod crypto;
pub mod provers;

#[async_trait]
pub trait ProofOfQuotaGenerator {
    async fn generate_poq(
        &self,
        public_inputs: &PublicInputs,
        key_index: u64,
    ) -> Result<(ProofOfQuota, ZkHash), quota::Error>;
}
