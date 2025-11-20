use crate::{
    Error, MessageIdentifier,
    crypto::{
        keys::{Ed25519PublicKey, X25519PrivateKey},
        proofs::{quota::VerifiedProofOfQuota, selection::inputs::VerifyInputs},
        signatures::Signature,
    },
    encap::{
        ProofsVerifier,
        decapsulated::{DecapsulatedMessage, DecapsulationOutput, PartDecapsulationOutput},
        encapsulated::{EncapsulatedMessage, EncapsulatedPart},
    },
    message::PublicHeader,
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
pub struct IncomingEncapsulatedMessageWithValidatedPublicHeader<const ENCAPSULATION_COUNT: usize> {
    validated_public_header: ValidatedPublicHeader,
    encapsulated_part: EncapsulatedPart<ENCAPSULATION_COUNT>,
}

impl<const ENCAPSULATION_COUNT: usize>
    IncomingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>
{
    pub(super) fn from_message(
        encapsulated_message: EncapsulatedMessage<ENCAPSULATION_COUNT>,
    ) -> Self {
        Self::from_message_unchecked(encapsulated_message)
    }

    /// Creates an instance of [`EncapsulatedMessage`] assuming its public
    /// header has been verified.
    ///
    /// This function should only be called from context where it's guaranteed
    /// the message has a valid public header.
    #[must_use]
    pub fn from_message_unchecked(
        encapsulated_message: EncapsulatedMessage<ENCAPSULATION_COUNT>,
    ) -> Self {
        let (public_header, encapsulated_part) = encapsulated_message.into_components();
        Self {
            encapsulated_part,
            validated_public_header: ValidatedPublicHeader::from_public_header_unchecked(
                public_header,
            ),
        }
    }

    #[must_use]
    pub fn into_components(self) -> (ValidatedPublicHeader, EncapsulatedPart<ENCAPSULATION_COUNT>) {
        (self.validated_public_header, self.encapsulated_part)
    }

    #[must_use]
    pub const fn id(&self) -> MessageIdentifier {
        self.validated_public_header.signing_pubkey
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
        let (validated_public_header, encapsulated_part) = self.into_components();
        let (_, signing_key, verified_proof_of_quota, _) =
            validated_public_header.into_components();

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
                    BlendingToken::new(verified_proof_of_quota, proof_of_selection);
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
                    BlendingToken::new(verified_proof_of_quota, proof_of_selection);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedPublicHeader {
    version: u8,
    proof_of_quota: VerifiedProofOfQuota,
    signing_pubkey: Ed25519PublicKey,
    signature: Signature,
}

impl From<ValidatedPublicHeader> for PublicHeader {
    fn from(
        ValidatedPublicHeader {
            proof_of_quota,
            signature,
            signing_pubkey,
            ..
        }: ValidatedPublicHeader,
    ) -> Self {
        Self::new(signing_pubkey, &proof_of_quota.into(), signature)
    }
}

impl ValidatedPublicHeader {
    #[must_use]
    pub const fn from_public_header_unchecked(public_header: PublicHeader) -> Self {
        let (version, signing_pubkey, proof_of_quota, signature) = public_header.into_components();

        Self {
            version,
            proof_of_quota: VerifiedProofOfQuota::from_proof_of_quota_unchecked(proof_of_quota),
            signing_pubkey,
            signature,
        }
    }

    #[must_use]
    pub const fn proof_of_quota(&self) -> &VerifiedProofOfQuota {
        &self.proof_of_quota
    }

    #[must_use]
    pub const fn into_components(self) -> (u8, Ed25519PublicKey, VerifiedProofOfQuota, Signature) {
        (
            self.version,
            self.signing_pubkey,
            self.proof_of_quota,
            self.signature,
        )
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
        self.0.id()
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

impl<const ENCAPSULATION_COUNT: usize>
    From<OutgoingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>>
    for EncapsulatedMessage<ENCAPSULATION_COUNT>
{
    fn from(
        OutgoingEncapsulatedMessageWithValidatedPublicHeader(
            incoming_message,
        ): OutgoingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>,
    ) -> Self {
        let (validated_public_header, encapsulated_part) = incoming_message.into_components();
        let public_header: PublicHeader = validated_public_header.into();
        Self::from_components(public_header, encapsulated_part)
    }
}
