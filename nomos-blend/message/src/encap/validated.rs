use crate::{
    Error, MessageIdentifier,
    crypto::{keys::X25519PrivateKey, proofs::selection::inputs::VerifyInputs},
    encap::{
        ProofsVerifier,
        decapsulated::{DecapsulatedMessage, DecapsulationOutput, PartDecapsulationOutput},
        encapsulated::EncapsulatedMessage,
    },
    reward::BlendingToken,
};

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(Default))]
/// Required inputs to verify a `PoSel` proof, minus the key nullifier that is
/// retrieved from the verified `PoQ` of the outer Blend layer.
pub struct RequiredProofOfSelectionVerificationInputs {
    pub expected_node_index: u64,
    pub total_membership_size: u64,
}

/// An incoming encapsulated Blend message whose public header has been
/// verified.
///
/// It can be decapsulated, but before being sent out as-is, it needs to be
/// converted into its outgoing variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingEncapsulatedMessageWithValidatedPublicHeader<const ENCAPSULATION_COUNT: usize>(
    EncapsulatedMessage<ENCAPSULATION_COUNT>,
);

impl<const ENCAPSULATION_COUNT: usize>
    IncomingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>
{
    pub(super) const fn from_message(
        encapsulated_message: EncapsulatedMessage<ENCAPSULATION_COUNT>,
    ) -> Self {
        Self(encapsulated_message)
    }

    #[must_use]
    pub const fn id(&self) -> MessageIdentifier {
        self.0.id()
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
        RequiredProofOfSelectionVerificationInputs {
            expected_node_index,
            total_membership_size,
        }: &RequiredProofOfSelectionVerificationInputs,
        verifier: &Verifier,
    ) -> Result<DecapsulationOutput<ENCAPSULATION_COUNT>, Error>
    where
        Verifier: ProofsVerifier,
    {
        let (public_header, encapsulated_part) = self.0.into_components();
        let (_, signing_key, verified_proof_of_quota, _) = public_header.into_components();

        // Derive the shared key.
        let shared_key = private_key.derive_shared_key(&signing_key.derive_x25519());

        // Decapsulate the encapsulated part.
        match encapsulated_part.decapsulate(
            &shared_key,
            &VerifyInputs {
                expected_node_index: *expected_node_index,
                key_nullifier: verified_proof_of_quota.key_nullifier(),
                total_membership_size: *total_membership_size,
            },
            verifier,
        )? {
            PartDecapsulationOutput::Incompleted {
                encapsulated_part,
                public_header,
                proof_of_selection,
            } => {
                let blending_token =
                    BlendingToken::new(*public_header.proof_of_quota(), proof_of_selection);
                Ok(DecapsulationOutput::Incompleted {
                    remaining_encapsulated_message: EncapsulatedMessage::from_components(
                        public_header,
                        encapsulated_part,
                    ),
                    blending_token,
                })
            }
            PartDecapsulationOutput::Completed {
                payload,
                proof_of_selection,
            } => {
                let (payload_type, payload_body) = payload.try_into_components()?;
                let blending_token =
                    BlendingToken::new(*public_header.proof_of_quota(), proof_of_selection);
                Ok(DecapsulationOutput::Completed {
                    fully_decapsulated_message: (DecapsulatedMessage::new(
                        payload_type,
                        payload_body,
                    )),
                    blending_token,
                })
            }
        }
    }
}

impl<const ENCAPSULATION_COUNT: usize>
    IncomingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>
{
    #[must_use]
    pub fn into_inner(self) -> EncapsulatedMessage<ENCAPSULATION_COUNT> {
        self.0
    }
}

/// An outgoing encapsulated Blend message whose public header has been
/// verified.
///
/// This message type does not offer any operations since it is only meant to be
/// used for outgoing messages that need serialization, hence it is used in
/// places where an `EncapsulatedMessage` is expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingEncapsulatedMessageWithValidatedPublicHeader<const ENCAPSULATION_COUNT: usize>(
    IncomingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>,
);

impl<const ENCAPSULATION_COUNT: usize>
    OutgoingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>
{
    #[must_use]
    pub const fn id(&self) -> MessageIdentifier {
        self.0.0.id()
    }
}

impl<const ENCAPSULATION_COUNT: usize>
    From<IncomingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>>
    for OutgoingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>
{
    fn from(
        value: IncomingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>,
    ) -> Self {
        Self(value)
    }
}

impl<const ENCAPSULATION_COUNT: usize> AsRef<EncapsulatedMessage<ENCAPSULATION_COUNT>>
    for OutgoingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>
{
    fn as_ref(&self) -> &EncapsulatedMessage<ENCAPSULATION_COUNT> {
        &self.0.0
    }
}
