use nomos_blend_message::crypto::proofs::quota::{self, ProofOfQuota, inputs::prove::PublicInputs};
use nomos_core::crypto::ZkHash;

pub mod crypto;
pub mod provers;

/// A component responsible for statelessly generating core variant `PoQ`s.
///
/// The trait provides the public context as well as the key index, while it
/// assumes the private info is known to the generator.
pub trait CoreProofOfQuotaGenerator {
    fn generate_poq(
        &self,
        public_inputs: &PublicInputs,
        key_index: u64,
    ) -> impl Future<Output = Result<(ProofOfQuota, ZkHash), quota::Error>> + Send + Sync;
}
