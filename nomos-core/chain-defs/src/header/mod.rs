use blake2::Digest as _;
use cryptarchia_engine::Slot;
use groth16::fr_to_bytes;
use serde::{Deserialize, Serialize};

pub const BEDROCK_VERSION: u8 = 1;

use crate::{
    crypto::Hasher,
    proofs::leader_proof::{Groth16LeaderProof, LeaderProof},
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
    parent_block: HeaderId,
    slot: Slot,
    block_root: ContentId,
    proof_of_leadership: Groth16LeaderProof,
}

impl Header {
    #[must_use]
    pub const fn parent(&self) -> HeaderId {
        self.parent_block
    }

    fn update_hasher(&self, h: &mut Hasher) {
        h.update(b"BLOCK_ID_V1");
        h.update([BEDROCK_VERSION]);
        h.update(self.parent_block.0);
        h.update(self.slot.to_le_bytes());
        h.update(self.block_root.0);
        h.update(self.proof_of_leadership.voucher_cm().to_bytes());
        h.update(fr_to_bytes(&self.proof_of_leadership.entropy()));
        h.update(self.proof_of_leadership.proof().to_bytes());
        h.update(self.proof_of_leadership.leader_key().to_bytes());
    }

    #[must_use]
    pub fn id(&self) -> HeaderId {
        let mut h = Hasher::new();
        self.update_hasher(&mut h);
        HeaderId(h.finalize().into())
    }

    #[must_use]
    pub fn leader_proof(&self) -> &impl LeaderProof {
        &self.proof_of_leadership
    }

    #[must_use]
    pub const fn slot(&self) -> Slot {
        self.slot
    }

    #[must_use]
    pub const fn new(
        parent_block: HeaderId,
        block_root: ContentId,
        slot: Slot,
        proof_of_leadership: Groth16LeaderProof,
    ) -> Self {
        Self {
            parent_block,
            slot,
            block_root,
            proof_of_leadership,
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
        <HeaderId as crate::codec::SerdeOp>::deserialize::<HeaderId>(
            &<HeaderId as crate::codec::SerdeOp>::serialize(&HeaderId([0; 32]))
                .expect("HeaderId should be able to be serialized")
        )
        .unwrap(),
        HeaderId([0; 32])
    );
}
