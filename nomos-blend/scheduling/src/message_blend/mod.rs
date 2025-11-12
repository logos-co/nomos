use nomos_blend_message::crypto::proofs::quota::{self, ProofOfQuota, inputs::prove::PublicInputs};
use nomos_core::crypto::ZkHash;

pub mod crypto;
pub mod provers;

pub trait ProofOfQuotaGenerator {
    fn generate_poq(
        &self,
        public_inputs: &PublicInputs,
        key_index: u64,
    ) -> impl Future<Output = Result<(ProofOfQuota, ZkHash), quota::Error>> + Send + Sync;
}
