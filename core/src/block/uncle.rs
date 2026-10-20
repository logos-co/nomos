use lb_binary_codec::canonical::BinaryCodec;
use lb_cryptarchia_engine::{MAX_UNCLES, Slot, UncleSlots};
// TODO(RFC-480): the proof of leadership is bound to the header by pol_msg, blocks carry no
// signature.
// use lb_key_management_system_keys::keys::Ed25519Signature;
use lb_utils::bounded::UpperBoundedVec;
use serde::{Deserialize, Serialize};

use crate::{
    // TODO(RFC-480): the proof of leadership is bound to the header by pol_msg, blocks carry no
    // signature.
    // block::Error,
    header::{Header, HeaderId},
};

/// Headers of the uncles referenced by a block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, BinaryCodec)]
pub struct UncleHeaders(UpperBoundedVec<Header, MAX_UNCLES>);

impl UncleHeaders {
    /// The maximum canonical representation of the bounded uncle list.
    pub const MAX_CANONICAL_ENCODED_SIZE: usize = 1 + MAX_UNCLES * Header::CANONICAL_ENCODED_SIZE;

    #[must_use]
    pub fn new(headers: impl Into<UpperBoundedVec<Header, MAX_UNCLES>>) -> Self {
        Self(headers.into())
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self(UpperBoundedVec::empty())
    }

    /// The slots the carried headers occupy.
    #[must_use]
    pub fn slots(&self) -> UncleSlots {
        let slots: Vec<Slot> = self.0.iter().map(Header::slot).collect();
        slots
            .try_into()
            .expect("one slot per header, so the same bound holds")
    }

    pub fn ids(&self) -> impl Iterator<Item = HeaderId> {
        self.0.iter().map(Header::id)
    }

    pub fn parents(&self) -> impl Iterator<Item = HeaderId> {
        self.0.iter().map(Header::parent)
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Header> {
        self.0.iter()
    }
}

// TODO(RFC-480): the proof of leadership is bound to the header by pol_msg,
// blocks carry no signature.
// /// A header together with the signature its leader produced over it.
// #[derive(
//     Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, BinaryCodec,
// )]
// pub struct SignedHeader {
//     header: Header,
//     signature: Ed25519Signature,
// }
//
// impl SignedHeader {
//     /// The fixed-size canonical representation of a signed header.
//     pub const CANONICAL_ENCODED_SIZE: usize = Header::CANONICAL_ENCODED_SIZE
//         + Ed25519Signature::CANONICAL_ENCODED_SIZE;
//
//     #[must_use]
//     pub const fn new(header: Header, signature: Ed25519Signature) -> Self {
//         Self { header, signature }
//     }
//
//     #[must_use]
//     pub const fn header(&self) -> &Header {
//         &self.header
//     }
//
//     #[must_use]
//     pub const fn signature(&self) -> &Ed25519Signature {
//         &self.signature
//     }
//
//     pub fn verify(&self) -> Result<(), Error> {
//         crate::block::verify_header_alone(&self.header)?;
//         crate::block::verify_header_signature(&self.header, &self.signature)
//     }
// }
