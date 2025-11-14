use groth16::Fr;
use nomos_blend_message::crypto::proofs::quota::{ProofOfQuota, inputs::prove::PublicInputs};
use poq::CorePathAndSelectors;
use zeroize::ZeroizeOnDrop;

/// A key that can be used within the Key Management Service.
pub trait SecuredKey: ZeroizeOnDrop {
    type Payload;
    type Signature;
    type PublicKey;
    type Error;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error>;
    fn sign_multiple(
        keys: &[&Self],
        payload: &Self::Payload,
    ) -> Result<Self::Signature, Self::Error>
    where
        Self: Sized;
    fn as_public_key(&self) -> Self::PublicKey;
    fn generate_core_poq(
        &self,
        public_inputs: &PublicInputs,
        key_index: u64,
        core_path_and_selectors: CorePathAndSelectors,
    ) -> Result<(ProofOfQuota, Fr), Self::Error>;
}
