use crate::{
    PayloadType,
    encap::encapsulated::{EncapsulatedMessage, EncapsulatedPart, EncapsulatedPrivateHeader},
    message::{Payload, PublicHeader},
};

/// The output of [`EncapsulatedMessage::decapsulate`]
#[expect(
    clippy::large_enum_variant,
    reason = "Size difference between variants is not too large (small ENCAPSULATION_COUNT)"
)]
#[derive(Clone)]
pub enum DecapsulationOutput<const ENCAPSULATION_COUNT: usize> {
    Incompleted(EncapsulatedMessage<ENCAPSULATION_COUNT>),
    Completed(DecapsulatedMessage),
}

/// The output of [`EncapsulatedPart::decapsulate`]
#[expect(
    clippy::large_enum_variant,
    reason = "Size difference between variants is not too large (small ENCAPSULATION_COUNT)"
)]
pub(super) enum PartDecapsulationOutput<const ENCAPSULATION_COUNT: usize> {
    Incompleted((EncapsulatedPart<ENCAPSULATION_COUNT>, PublicHeader)),
    Completed(Payload),
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
pub(super) enum PrivateHeaderDecapsulationOutput<const ENCAPSULATION_COUNT: usize> {
    Incompleted((EncapsulatedPrivateHeader<ENCAPSULATION_COUNT>, PublicHeader)),
    Completed((EncapsulatedPrivateHeader<ENCAPSULATION_COUNT>, PublicHeader)),
}
