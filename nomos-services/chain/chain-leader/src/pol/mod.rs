pub mod merkle;

use std::sync::LazyLock;

use cryptarchia_engine::Slot;
use groth16::{Fr, fr_from_bytes, fr_from_bytes_unchecked, fr_to_bytes};
use nomos_core::crypto::{ZkDigest, ZkHash, ZkHasher};
use rs_merkle_tree::stores::MemoryStore;
use zeroize::Zeroize;

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

const MAX_TREE_DEPTH: u8 = 25;
const TREE_LEAF_COUNT: usize = 2 << MAX_TREE_DEPTH;
