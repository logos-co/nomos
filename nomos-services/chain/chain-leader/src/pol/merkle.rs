use blake2::{Blake2b512, Digest as _};
use cryptarchia_engine::Slot;
use groth16::{Fr, fr_from_bytes_unchecked, fr_to_bytes};
use nomos_core::{
    crypto::{ZkDigest, ZkHasher},
    utils::merkle::{MerkleNode, MerklePath},
};
use nomos_utils::blake_rng::{BlakeRng, BlakeRngSeed, SeedableRng as _};
use rand::RngCore as _;
use rayon::iter::{IntoParallelIterator as _, IntoParallelRefIterator as _, ParallelIterator as _};

use crate::pol::SlotSecret;

pub type CachedTree = Vec<Vec<Fr>>;

#[derive(Clone)]
pub struct MerklePolCache {
    pub cached_tree: CachedTree,
    pub lower_sub_trees: Vec<MerklePolSubtree>,
    pub tree_depth: usize,
    pub starting_slot: Slot,
}

impl MerklePolCache {
    #[must_use]
    pub fn new(
        seed: BlakeRngSeed,
        starting_slot: Slot,
        tree_depth: usize,
        cache_depth: usize,
    ) -> Self {
        let sub_tree_depth = tree_depth - cache_depth;
        let numer_of_sub_trees = 2usize.pow(cache_depth as u32);
        // we have to pre-cache the leaves so we can parallelize the subtree computation
        let lower_sub_trees: Vec<_> = Self::leaves_from_seed(seed)
            .take(numer_of_sub_trees)
            .map(|seed| MerklePolSubtree::new(seed, sub_tree_depth))
            .collect();

        let lower_tree_roots: Vec<Fr> = lower_sub_trees
            .par_iter()
            .map(|subtree| {
                *subtree
                    .merkle_path_for_index(0)
                    .first()
                    .map(MerkleNode::item)
                    .expect("Root should be always present")
            })
            .collect();

        // compute the root slot secret
        let cached_tree = compute_cached_tree_from_leafs(&lower_tree_roots);

        Self {
            cached_tree,
            lower_sub_trees,
            tree_depth,
            starting_slot,
        }
    }

    pub fn leaves_from_seed(seed: BlakeRngSeed) -> impl Iterator<Item = Fr> {
        let mut rng = BlakeRng::from_seed(seed);
        std::iter::repeat_with(move || {
            let mut bytes = [0u8; 31];
            rng.fill_bytes(&mut bytes);
            fr_from_bytes_unchecked(&bytes)
        })
    }

    #[must_use]
    pub fn root_slot_secret(&self) -> SlotSecret {
        // this should always be there
        self.cached_tree[0][0].into()
    }

    #[must_use]
    pub const fn cache_depth(&self) -> usize {
        self.cached_tree.len()
    }

    #[must_use]
    pub fn merkle_path_for_index(&self, index: usize) -> MerklePath<Fr> {
        let mut cached_path = get_merkle_path(&self.cached_tree, index, self.cached_tree.len());
        let subtree_leaf_length = 2usize.pow((self.tree_depth - self.cache_depth()) as u32);
        let subtree_index = index / subtree_leaf_length;
        let leaf_index = index % subtree_leaf_length;
        let current_path = self.lower_sub_trees[subtree_index].merkle_path_for_index(leaf_index);
        cached_path.extend(current_path);
        cached_path
    }
}

#[derive(Clone)]
pub struct MerklePolSubtree {
    seed: Fr,
    tree_depth: usize,
}

impl MerklePolSubtree {
    #[must_use]
    pub const fn new(seed: Fr, tree_depth: usize) -> Self {
        Self { seed, tree_depth }
    }

    #[must_use]
    pub fn merkle_path_for_index(&self, index: usize) -> MerklePath<Fr> {
        let hashed_leafs: Vec<_> = std::iter::successors(Some(self.seed), |seed| {
            Some(fr_from_bytes_unchecked(
                &Blake2b512::digest(fr_to_bytes(seed))[..31],
            ))
        })
        .take(2usize.pow(self.tree_depth as u32))
        .collect();
        let merkle_tree = compute_cached_tree_from_leafs(&hashed_leafs);
        get_merkle_path(&merkle_tree, index, self.depth())
    }

    #[must_use]
    pub const fn depth(&self) -> usize {
        self.tree_depth
    }
}

fn compute_cached_tree_from_leafs(tree_leafs: &[Fr]) -> Vec<Vec<Fr>> {
    let mut cached_tree: Vec<Vec<Fr>> = std::iter::successors(Some(tree_leafs.to_vec()), |leafs| {
        if leafs.len() <= 1 {
            return None;
        }
        let roots_chunks: Vec<&[Fr]> = leafs.chunks(2).collect();
        Some(
            roots_chunks
                .into_par_iter()
                .map(|pair| <[Fr; 2]>::try_from(pair).unwrap())
                .map(|pair| <ZkHasher as ZkDigest>::compress(&pair))
                .collect(),
        )
    })
    .collect();
    cached_tree.reverse();
    cached_tree
}

#[must_use]
pub fn get_merkle_path(
    cached_tree: &CachedTree,
    mut current_index: usize,
    tree_depth: usize,
) -> MerklePath<Fr> {
    let mut path = MerklePath::new();

    for level in (0..tree_depth).rev() {
        let sibling_index = if current_index.is_multiple_of(2) {
            current_index + 1
        } else {
            current_index - 1
        };

        if sibling_index < cached_tree[level].len() {
            let value = cached_tree[level][sibling_index];
            let node = if sibling_index.is_multiple_of(2) {
                MerkleNode::Left(value)
            } else {
                MerkleNode::Right(value)
            };
            path.push(node);
        }

        current_index /= 2;
    }
    path.reverse();
    path
}
