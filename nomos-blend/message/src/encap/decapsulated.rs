use crate::{
    PayloadType,
    crypto::proofs::selection::ProofOfSelection,
    encap::encapsulated::{EncapsulatedMessage, EncapsulatedPart, EncapsulatedPrivateHeader},
    message::{Payload, PublicHeader},
    reward::BlendingToken,
};

/// The output of [`EncapsulatedMessage::decapsulate`]
#[derive(Clone)]
pub enum DecapsulationOutput {
    Incompleted {
        remaining_encapsulated_message: Box<EncapsulatedMessage>,
        blending_token: BlendingToken,
    },
    Completed {
        fully_decapsulated_message: DecapsulatedMessage,
        blending_token: BlendingToken,
    },
}

/// The output of [`EncapsulatedPart::decapsulate`]
pub(super) enum PartDecapsulationOutput {
    Incompleted {
        encapsulated_part: EncapsulatedPart,
        public_header: Box<PublicHeader>,
        proof_of_selection: ProofOfSelection,
    },

    Completed {
        payload: Payload,
        proof_of_selection: ProofOfSelection,
    },
}

#[derive(Clone, Debug)]
pub struct DecapsulatedMessage {
    payload_type: PayloadType,
    payload_body: Vec<u8>,
}

impl DecapsulatedMessage {
    pub(crate) const fn new(payload_type: PayloadType, payload_body: Vec<u8>) -> Self {
        Self {
            payload_type,
            payload_body,
        }
    }

    #[must_use]
    pub const fn payload_type(&self) -> PayloadType {
        self.payload_type
    }

    #[must_use]
    pub fn payload_body(&self) -> &[u8] {
        &self.payload_body
    }

    #[must_use]
    pub fn into_components(self) -> (PayloadType, Vec<u8>) {
        (self.payload_type, self.payload_body)
    }
}

/// The output of [`EncapsulatedPrivateHeader::decapsulate`]
pub(super) enum PrivateHeaderDecapsulationOutput {
    Incompleted {
        encapsulated_private_header: EncapsulatedPrivateHeader,
        public_header: PublicHeader,
        proof_of_selection: ProofOfSelection,
    },
    Completed {
        encapsulated_private_header: EncapsulatedPrivateHeader,
        public_header: PublicHeader,
        proof_of_selection: ProofOfSelection,
    },
}
