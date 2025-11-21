use std::sync::LazyLock;

use cryptarchia_engine::Slot;
use ed25519_dalek::ed25519::signature::digest::Mac;
use groth16::{Fr, fr_from_bytes};
use mmr::MerkleMountainRange;
use nomos_core::{
    crypto::{ZkDigest, ZkHash, ZkHasher},
    utils::merkle::MerklePath,
};
use zeroize::Zeroize;

#[derive(Zeroize)]
pub struct SlotSecret(Fr);

impl AsRef<Fr> for SlotSecret {
    fn as_ref(&self) -> &Fr {
        &self.0
    }
}

impl From<Fr> for SlotSecret {
    fn from(fr: Fr) -> Self {
        Self(fr)
    }
}

type Mmr = MerkleMountainRange<SlotSecret, ZkHasher, 25>;

static NOMOS_POL_SK_V1: LazyLock<Fr> = LazyLock::new(|| {
    fr_from_bytes(b"NOMOS_POL_SK_V1").expect("NOMOS_POL_SK_V1 should be a valid Fr")
});

const MAX_TREE_DEPTH: u8 = 25;
pub struct PolSecretKey {
    current_slot: Slot,
    mmr: Mmr,
}

pub fn pol_sk_generator(slot_start: Slot, seed: ZkHash) -> (SlotSecret, MerklePath<ZkHash>) {
    let mut frontier_nodes: Mmr = Mmr::new();
    let mut path = MerklePath::new();
    let mut slot_secret = seed.into();

    for i in 0..2u64.pow(u32::from(MAX_TREE_DEPTH)) {
        frontier_nodes = frontier_nodes.push(slot_secret);
        path.push(nomos_core::utils::merkle::MerkleNode::Right(
            frontier_nodes.frontier_root(),
        ));
        slot_secret = ZkHasher::digest(&[seed]).into();
    }

    let root = frontier_nodes.frontier_root();
    let slot_start_fr = fr_from_bytes(slot_start.to_le_bytes().as_slice())
        .expect("Slot should be a valid Fr range");

    (
        SlotSecret(ZkHasher::digest(&[*NOMOS_POL_SK_V1, slot_start_fr, root])),
        path,
    )
}
