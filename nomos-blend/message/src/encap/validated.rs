use core::ops::Deref;

use nomos_core::crypto::ZkHash;

use crate::{
    crypto::{keys::X25519PrivateKey, proofs::selection::inputs::VerifyInputs},
    encap::{
        decapsulated::{DecapsulatedMessage, DecapsulationOutput, PartDecapsulationOutput},
        encapsulated::EncapsulatedMessage,
        ProofsVerifier,
    },
    Error,
};

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(Default))]
/// Required inputs to verify a `PoSel` proof, minus the key nullifier that is
/// retrieved from the verified `PoQ` of the outer Blend layer.
pub struct MissingProofOfSelectionVerificationInputs {
    pub expected_node_index: u64,
    pub total_membership_size: u64,
}

/// An encapsulated Blend message whose public header has been verified but not
/// unwrapped.
#[derive(Debug, PartialEq, Eq)]
pub struct EncapsulatedMessageWithValidatedPublicHeader<const ENCAPSULATION_COUNT: usize> {
    /// Key nullifier as returned by the verified `PoQ` of the verified public
    /// header.
    key_nullifier: ZkHash,
    encapsulated_message: EncapsulatedMessage<ENCAPSULATION_COUNT>,
}

impl<const ENCAPSULATION_COUNT: usize>
    EncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>
{
    pub(super) const fn from_components(
        encapsulated_message: EncapsulatedMessage<ENCAPSULATION_COUNT>,
        key_nullifier: ZkHash,
    ) -> Self {
        Self {
            key_nullifier,
            encapsulated_message,
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
        let (public_header, encapsulated_part) = self.encapsulated_message.into_components();

        // Derive the shared key.
        let shared_key =
            private_key.derive_shared_key(&public_header.signing_pubkey().derive_x25519());

        // Decapsulate the encapsulated part.
        match encapsulated_part.decapsulate(
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

    #[cfg(any(test, feature = "unsafe-test-functions"))]
    #[must_use]
    /// Wraps an `EncapsulatedMessage` into a
    /// `EncapsulatedMessageWithValidatedPublicHeader` using a zero key
    /// nullifier.
    ///
    /// This function should not be used in production, and should only be used
    /// for tests.
    pub const fn from_encapsulated_message_unchecked(
        encapsulated_message: EncapsulatedMessage<ENCAPSULATION_COUNT>,
    ) -> Self {
        use groth16::Field as _;

        Self {
            encapsulated_message,
            key_nullifier: ZkHash::ZERO,
        }
    }
}

impl<const ENCAPSULATION_COUNT: usize>
    EncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>
{
    #[must_use]
    pub fn into_components(self) -> (EncapsulatedMessage<ENCAPSULATION_COUNT>, ZkHash) {
        (self.encapsulated_message, self.key_nullifier)
    }
}

impl<const ENCAPSULATION_COUNT: usize> Deref
    for EncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>
{
    type Target = EncapsulatedMessage<ENCAPSULATION_COUNT>;

    fn deref(&self) -> &Self::Target {
        &self.encapsulated_message
    }
}
