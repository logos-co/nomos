use crate::{
    crypto::{
        keys::{Ed25519PublicKey, X25519PrivateKey},
        proofs::quota::inputs::prove::PublicInputs,
    },
    encap::{
        decapsulated::{DecapsulatedMessage, DecapsulationOutput, PartDecapsulationOutput},
        encapsulated::{EncapsulatedMessage, EncapsulatedPart},
    },
    Error,
};

pub struct UnwrappedEncapsulatedMessage<const ENCAPSULATION_COUNT: usize> {
    public_inputs: PublicInputs,
    signing_public_key: Ed25519PublicKey,
    /// Encapsulated parts
    encapsulated_part: EncapsulatedPart<ENCAPSULATION_COUNT>,
}

impl<const ENCAPSULATION_COUNT: usize> UnwrappedEncapsulatedMessage<ENCAPSULATION_COUNT> {
    pub(super) const fn new(
        public_inputs: &PublicInputs,
        signing_public_key: Ed25519PublicKey,
        encapsulated_part: EncapsulatedPart<ENCAPSULATION_COUNT>,
    ) -> Self {
        Self {
            public_inputs: *public_inputs,
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
    pub fn decapsulate(
        self,
        private_key: &X25519PrivateKey,
    ) -> Result<DecapsulationOutput<ENCAPSULATION_COUNT>, Error> {
        // Derive the shared key.
        let shared_key = private_key.derive_shared_key(&self.signing_public_key.derive_x25519());

        // Decapsulate the encapsulated part.
        match self
            .encapsulated_part
            .decapsulate(&shared_key, &self.public_inputs)?
        {
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
