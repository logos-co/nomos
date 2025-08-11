use core::ops::{Deref, DerefMut};

use nomos_blend_message::Error;
use nomos_blend_scheduling::EncapsulatedMessage;

/// An encapsulated message whose public header as been validated according to
/// the Blend specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncapsulatedMessageWithValidatedPublicHeader(EncapsulatedMessage);

impl EncapsulatedMessageWithValidatedPublicHeader {
    #[must_use]
    pub fn into_inner(self) -> EncapsulatedMessage {
        self.0
    }
}

impl Deref for EncapsulatedMessageWithValidatedPublicHeader {
    type Target = EncapsulatedMessage;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for EncapsulatedMessageWithValidatedPublicHeader {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub trait ValidateMessagePublicHeader {
    fn validate_public_header(self) -> Result<EncapsulatedMessageWithValidatedPublicHeader, Error>;
}

impl ValidateMessagePublicHeader for EncapsulatedMessage {
    fn validate_public_header(self) -> Result<EncapsulatedMessageWithValidatedPublicHeader, Error> {
        self.verify_public_header()?;
        Ok(EncapsulatedMessageWithValidatedPublicHeader(self))
    }
}
