use blake2::Digest as _;
use cryptarchia_engine::Slot;
use ed25519_dalek::Signer as _;
use serde::{Deserialize, Serialize};

pub const BEDROCK_VERSION: u8 = 1;

use crate::{
    crypto::Hasher,
    proofs::leader_proof::{LeaderProof, Risc0LeaderProof},
    utils::{display_hex_bytes_newtype, serde_bytes_newtype},
};
use crate::codec::SerdeOp;

#[derive(Clone, Debug, Eq, PartialEq, Copy, Hash, PartialOrd, Ord)]
pub struct HeaderId([u8; 32]);

#[derive(Clone, Debug, Eq, PartialEq, Copy, Hash)]
pub struct ContentId([u8; 32]);

#[derive(Clone, Debug, Eq, PartialEq, Copy)]
pub struct Nonce([u8; 32]);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub proposal_version: u8,
    pub cryptarchia_version: u8,
    pub mantle_version: u8,
}

impl Default for Version {
    fn default() -> Self {
        Self {
            proposal_version: BEDROCK_VERSION,
            cryptarchia_version: BEDROCK_VERSION,
            mantle_version: BEDROCK_VERSION,
        }
    }
}

impl Version {
    #[must_use]
    pub const fn to_bytes(&self) -> [u8; 3] {
        [
            self.proposal_version,
            self.cryptarchia_version,
            self.mantle_version,
        ]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    version: Version,
    parent_block: HeaderId,
    slot: Slot,
    block_root: ContentId,
    proof_of_leadership: Risc0LeaderProof,
}

impl Header {
    #[must_use]
    pub const fn version(&self) -> &Version {
        &self.version
    }

    #[must_use]
    pub const fn is_valid_bedrock_version(&self) -> bool {
        self.version.proposal_version == BEDROCK_VERSION
            && self.version.cryptarchia_version == BEDROCK_VERSION
            && self.version.mantle_version == BEDROCK_VERSION
    }

    #[must_use]
    pub const fn parent(&self) -> HeaderId {
        self.parent_block
    }

    fn update_hasher(&self, h: &mut Hasher) {
        h.update(b"BLOCK_ID_V1");
        h.update(self.version.to_bytes());
        h.update(self.parent_block.0);
        h.update(self.slot.to_le_bytes());
        h.update(self.block_root.0);
        // TODO: add leader voucher
        h.update(self.proof_of_leadership.entropy()); // TODO: blake2b to Fr
                                                      // TODO: serialize proof
                                                      // TODO: add leader key
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

    pub fn sign(
        &self,
        signing_key: &ed25519_dalek::SigningKey,
    ) -> Result<ed25519_dalek::Signature, crate::block::Error> {
        let header_bytes = <Self as SerdeOp>::serialize(self)?;
        Ok(signing_key.sign(&header_bytes))
    }

    #[must_use]
    pub fn new(
        parent_block: HeaderId,
        block_root: ContentId,
        slot: Slot,
        proof_of_leadership: Risc0LeaderProof,
    ) -> Self {
        Self {
            version: Version::default(),
            parent_block,
            slot,
            block_root,
            proof_of_leadership,
        }
    }

    #[must_use]
    pub const fn new_with_version(
        version: Version,
        parent_block: HeaderId,
        block_root: ContentId,
        slot: Slot,
        proof_of_leadership: Risc0LeaderProof,
    ) -> Self {
        Self {
            version,
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
