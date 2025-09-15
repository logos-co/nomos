use core::ops::Deref;

use nomos_core::crypto::ZkHash;

use crate::encap::encapsulated::EncapsulatedMessage;

/// An encapsulated Blend message whose public header has been verified.
#[derive(Debug)]
pub struct ValidatedEncapsulatedMessage<const ENCAPSULATION_COUNT: usize> {
    encapsulated_message: EncapsulatedMessage<ENCAPSULATION_COUNT>,
    key_nullifier: ZkHash,
}

impl<const ENCAPSULATION_COUNT: usize> ValidatedEncapsulatedMessage<ENCAPSULATION_COUNT> {
    pub(super) const fn from_components(
        encapsulated_message: EncapsulatedMessage<ENCAPSULATION_COUNT>,
        key_nullifier: ZkHash,
    ) -> Self {
        Self {
            encapsulated_message,
            key_nullifier,
        }
    }

    #[cfg(any(test, feature = "unsafe-test-functions"))]
    #[must_use]
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

impl<const ENCAPSULATION_COUNT: usize> ValidatedEncapsulatedMessage<ENCAPSULATION_COUNT> {
    #[must_use]
    pub fn into_components(self) -> (EncapsulatedMessage<ENCAPSULATION_COUNT>, ZkHash) {
        (self.encapsulated_message, self.key_nullifier)
    }
}

impl<const ENCAPSULATION_COUNT: usize> Deref for ValidatedEncapsulatedMessage<ENCAPSULATION_COUNT> {
    type Target = EncapsulatedMessage<ENCAPSULATION_COUNT>;

    fn deref(&self) -> &Self::Target {
        &self.encapsulated_message
    }
}
