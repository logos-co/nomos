use nomos_core::crypto::ZkHash;

use crate::{
    crypto::{
        keys::{Ed25519PublicKey, X25519PrivateKey},
        proofs::selection::inputs::VerifyInputs,
    },
    encap::{
        decapsulated::{DecapsulatedMessage, DecapsulationOutput, PartDecapsulationOutput},
        encapsulated::{EncapsulatedMessage, EncapsulatedPart},
        ProofsVerifier,
    },
    Error,
};

/// A Blend message whose public header has been verified.
pub struct UnwrappedEncapsulatedMessage<const ENCAPSULATION_COUNT: usize> {
    /// Key nullifier as returned by the verified `PoQ` of the unwrapped public
    /// header.
    key_nullifier: ZkHash,
    /// The public key that signed the Blend message and whose validity has been
    /// verified.
    signing_public_key: Ed25519PublicKey,
    /// The remaining, unverified, encapsulated message component.
    encapsulated_part: EncapsulatedPart<ENCAPSULATION_COUNT>,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(Default))]
/// Required inputs to verify a `PoSel` proof, minus the key nullifier that is
/// retrieved from the verified `PoQ` of the outer Blend layer.
pub struct MissingProofOfSelectionVerificationInputs {
    pub expected_node_index: u64,
    pub total_membership_size: u64,
}

impl<const ENCAPSULATION_COUNT: usize> UnwrappedEncapsulatedMessage<ENCAPSULATION_COUNT> {
    pub(super) const fn new(
        key_nullifier: ZkHash,
        signing_public_key: Ed25519PublicKey,
        encapsulated_part: EncapsulatedPart<ENCAPSULATION_COUNT>,
    ) -> Self {
        Self {
            key_nullifier,
            signing_public_key,
            encapsulated_part,
        }
    }

    /// Decapsulates the message using the provided key.
    ///
    /// If the provided key is eligible, returns the following:
    /// - [`DecapsulationOutput::Completed`] if the message was fully
    ///   decapsulated by this call.
    /// - [`DecapsulationOutput::Incompleted`] if the message is still
    ///   encapsulated.
    ///
    /// If not, [`Error::DeserializationFailed`] or
    /// [`Error::ProofOfSelectionVerificationFailed`] will be returned.
    pub fn decapsulate<Verifier>(
        self,
        private_key: &X25519PrivateKey,
        MissingProofOfSelectionVerificationInputs {
            expected_node_index,
            total_membership_size,
        }: &MissingProofOfSelectionVerificationInputs,
        verifier: &Verifier,
    ) -> Result<DecapsulationOutput<ENCAPSULATION_COUNT>, Error>
    where
        Verifier: ProofsVerifier,
    {
        // Derive the shared key.
        let shared_key = private_key.derive_shared_key(&self.signing_public_key.derive_x25519());

        // Decapsulate the encapsulated part.
        match self.encapsulated_part.decapsulate(
            &shared_key,
            &VerifyInputs {
                expected_node_index: *expected_node_index,
                key_nullifier: self.key_nullifier,
                total_membership_size: *total_membership_size,
            },
            verifier,
        )? {
            PartDecapsulationOutput::Incompleted((encapsulated_part, public_header)) => {
                Ok(DecapsulationOutput::Incompleted(
                    EncapsulatedMessage::from_components(public_header, encapsulated_part),
                ))
            }
            PartDecapsulationOutput::Completed(payload) => {
                let (payload_type, payload_body) = payload.try_into_components()?;
                Ok(DecapsulationOutput::Completed(DecapsulatedMessage::new(
                    payload_type,
                    payload_body,
                )))
            }
        }
    }
}
