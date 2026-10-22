use lb_binary_codec::canonical::BinaryCodec;
use lb_cryptarchia_engine::{MAX_UNCLES, Slot, UncleSlots};
use lb_utils::bounded::UpperBoundedVec;
use serde::{Deserialize, Serialize};

use crate::header::{Header, HeaderId};

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
