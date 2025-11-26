pub mod merkle;

use std::sync::LazyLock;

use cryptarchia_engine::Slot;
use ed25519_dalek::ed25519::signature::digest::Mac;
use groth16::{Fr, fr_from_bytes, fr_from_bytes_unchecked, fr_to_bytes};
use mmr::MerkleMountainRange;
use nomos_core::{
    crypto::{ZkDigest, ZkHash, ZkHasher},
    utils::merkle::{MerkleNode, MerklePath},
};
use rs_merkle_tree::stores::MemoryStore;
use zeroize::Zeroize;

use crate::pol::merkle::Mmr;

#[derive(Zeroize, Copy, Clone)]
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

static NOMOS_POL_SK_V1: LazyLock<Fr> = LazyLock::new(|| {
    fr_from_bytes(b"NOMOS_POL_SK_V1").expect("NOMOS_POL_SK_V1 should be a valid Fr")
});

const MAX_TREE_DEPTH: u8 = 25;
const TREE_LEAF_COUNT: usize = 2 << MAX_TREE_DEPTH;
pub struct PolSecretKey {
    current_slot: Slot,
    mmr: Mmr,
}

// #[must_use]
// pub fn pol_sk_generator(slot_start: Slot, seed: ZkHash) -> (SlotSecret,
// Vec<Fr>) {     let mut frontier_nodes: Mmr = Mmr::new();
//     // let mut frontier_vec = Vec::with_capacity();
//     // let mut path = MerklePath::new();
//     let depth = 2u64.pow(u32::from(MAX_TREE_DEPTH));
//     let mut nodes = Vec::with_capacity(depth.try_into().unwrap());
//     let mut slot_secret: SlotSecret = seed.into();
//
//     for i in 0..depth {
//         frontier_nodes = frontier_nodes.push(slot_secret);
//         nodes.push(frontier_nodes.frontier_root());
//
//         slot_secret = ZkHasher::digest(&[slot_secret.0]).into();
//     }
//
//     let root = frontier_nodes.frontier_root();
//     let slot_start_fr = fr_from_bytes(slot_start.to_le_bytes().as_slice())
//         .expect("Slot should be a valid Fr range");
//
//     (
//         SlotSecret(ZkHasher::digest(&[*NOMOS_POL_SK_V1, slot_start_fr,
// root])),         nodes,
//     )
// }

pub struct InnerTreeZkHasher;

impl rs_merkle_tree::hasher::Hasher for InnerTreeZkHasher {
    fn hash(
        &self,
        left: &rs_merkle_tree::node::Node,
        right: &rs_merkle_tree::node::Node,
    ) -> rs_merkle_tree::node::Node {
        let mut hasher = ZkHasher::new();
        hasher.compress(&[
            // We use `unchecked` because we control the inputs, and poseidon hasher is guaranteed
            // to always output valid `Fr` points.
            fr_from_bytes_unchecked(left.as_ref()),
            fr_from_bytes_unchecked(right.as_ref()),
        ]);
        fr_to_bytes(&hasher.finalize()).into()
    }
}

#[must_use]
pub fn pol_sk_generator(
    slot_start: Slot,
    seed: ZkHash,
) -> rs_merkle_tree::MerkleTree<InnerTreeZkHasher, MemoryStore, 24> {
    let mut merkle_tree: rs_merkle_tree::MerkleTree<InnerTreeZkHasher, MemoryStore, 24> =
        rs_merkle_tree::MerkleTree::new(InnerTreeZkHasher, MemoryStore::new());

    let depth: usize = 2u64.pow(u32::from(MAX_TREE_DEPTH)).try_into().unwrap();

    let leafs: Vec<rs_merkle_tree::node::Node> =
        std::iter::successors(Some(seed), |&seed| Some(ZkHasher::digest(&[seed])))
            .take(depth)
            .map(|fr| fr_to_bytes(&fr).into())
            .collect();

    merkle_tree
        .add_leaves(&leafs)
        .expect("Size is previously checked, this should not fail");

    merkle_tree
}
