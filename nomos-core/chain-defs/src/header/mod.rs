use blake2::Digest as _;
use cryptarchia_engine::Slot;
use serde::{Deserialize, Serialize};

use crate::{
    crypto::Hasher,
    proofs::leader_proof::{LeaderProof, Risc0LeaderProof},
    utils::{display_hex_bytes_newtype, serde_bytes_newtype},
};

#[derive(Clone, Debug, Eq, PartialEq, Copy, Hash, PartialOrd, Ord)]
pub struct HeaderId([u8; 32]);

#[derive(Clone, Debug, Eq, PartialEq, Copy, Hash)]
pub struct ContentId([u8; 32]);

#[derive(Clone, Debug, Eq, PartialEq, Copy)]
pub struct Nonce([u8; 32]);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    parent: HeaderId,
    slot: Slot,
    // TODO: move this to common header fields
    // length of block contents in bytes
    content_size: u32,
    // id of block contents
    content_id: ContentId,
    leader_proof: Risc0LeaderProof,
    orphaned_leader_proofs: Vec<Header>,
}

impl Header {
    #[must_use]
    pub const fn parent(&self) -> HeaderId {
        self.parent
    }

    fn update_hasher(&self, h: &mut Hasher) {
        h.update(b"\x01");
        h.update(self.content_size.to_be_bytes());
        h.update(self.content_id.0);
        h.update(self.slot.to_be_bytes());
        h.update(self.parent.0);

        // TODO: header hash missing in specs
    }

    #[must_use]
    pub fn id(&self) -> HeaderId {
        let mut h = Hasher::new();
        self.update_hasher(&mut h);
        HeaderId(h.finalize().into())
    }

    #[must_use]
    pub fn leader_proof(&self) -> &impl LeaderProof {
        &self.leader_proof
    }

    #[must_use]
    pub const fn slot(&self) -> Slot {
        self.slot
    }

    #[must_use]
    pub fn orphaned_proofs(&self) -> &[Self] {
        &self.orphaned_leader_proofs
    }

    #[must_use]
    pub const fn content_size(&self) -> u32 {
        self.content_size
    }

    #[must_use]
    pub const fn new(
        parent: HeaderId,
        content_size: u32,
        content_id: ContentId,
        slot: Slot,
        leader_proof: Risc0LeaderProof,
    ) -> Self {
        Self {
            parent,
            content_size,
            content_id,
            slot,
            leader_proof,
            orphaned_leader_proofs: vec![],
        }
    }

    #[must_use]
    pub fn with_orphaned_proofs(mut self, orphaned_leader_proofs: Vec<Self>) -> Self {
        self.orphaned_leader_proofs = orphaned_leader_proofs;
        self
    }
}

pub struct Builder {
    parent: HeaderId,
    slot: Slot,
    leader_proof: Risc0LeaderProof,
    orphaned_leader_proofs: Vec<Header>,
}

impl Builder {
    #[must_use]
    pub const fn new(parent: HeaderId, slot: Slot, leader_proof: Risc0LeaderProof) -> Self {
        Self {
            parent,
            slot,
            leader_proof,
            orphaned_leader_proofs: vec![],
        }
    }

    #[must_use]
    pub fn with_orphaned_proofs(mut self, orphaned_leader_proofs: Vec<Header>) -> Self {
        self.orphaned_leader_proofs = orphaned_leader_proofs;
        self
    }

    #[must_use]
    pub fn build(self, content_id: ContentId, content_size: u32) -> Header {
        Header {
            parent: self.parent,
            slot: self.slot,
            content_size,
            content_id,
            leader_proof: self.leader_proof,
            orphaned_leader_proofs: self.orphaned_leader_proofs,
        }
    }
}

impl From<[u8; 32]> for HeaderId {
    fn from(id: [u8; 32]) -> Self {
        Self(id)
    }
}

impl From<HeaderId> for [u8; 32] {
    fn from(id: HeaderId) -> Self {
        id.0
    }
}

impl TryFrom<&[u8]> for HeaderId {
    type Error = Error;

    fn try_from(slice: &[u8]) -> Result<Self, Self::Error> {
        if slice.len() != 32 {
            return Err(Error::InvalidHeaderIdSize(slice.len()));
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(slice);
        Ok(Self::from(id))
    }
}

impl From<[u8; 32]> for ContentId {
    fn from(id: [u8; 32]) -> Self {
        Self(id)
    }
}

impl From<ContentId> for [u8; 32] {
    fn from(id: ContentId) -> Self {
        id.0
    }
}

display_hex_bytes_newtype!(HeaderId);
display_hex_bytes_newtype!(ContentId);

serde_bytes_newtype!(HeaderId, 32);
serde_bytes_newtype!(ContentId, 32);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid header id size: {0}")]
    InvalidHeaderIdSize(usize),
}

#[test]
fn test_serde() {
    assert_eq!(
        crate::wire::deserialize::<HeaderId>(&crate::wire::serialize(&HeaderId([0; 32])).unwrap())
            .unwrap(),
        HeaderId([0; 32])
    );
}
