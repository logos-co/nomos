use std::collections::VecDeque;

use cryptarchia_engine::Slot;
use groth16::{Fr, fr_from_bytes, fr_from_bytes_unchecked};
use nomos_core::crypto::{ZkDigest, ZkHasher};
use rand::{RngCore, random};
use rayon::iter::{
    IntoParallelIterator as _, IntoParallelRefIterator, ParallelExtend as _, ParallelIterator as _,
};

use crate::pol::SlotSecret;

pub type CachedTree = Vec<Vec<Fr>>;

#[derive(Clone)]
pub struct MerklePolCache {
    pub cached_tree: CachedTree,
    pub lower_sub_trees: Vec<MerklePolSubtree>,
    pub tree_depth: usize,
    pub current_index: u64,
    pub starting_slot: Slot,
}

impl MerklePolCache {
    #[must_use]
    pub fn new(seed: Fr, starting_slot: Slot, tree_depth: usize, cache_depth: usize) -> Self {
        let sub_tree_depth = tree_depth - cache_depth;
        let numer_of_sub_trees = 2usize.pow(sub_tree_depth as u32);
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
            current_index: 0,
        }
    }

    pub fn leaves_from_seed(seed: Fr) -> impl Iterator<Item = Fr> {
        std::iter::repeat_with(|| fr_from_bytes_unchecked(&random::<[u8; 31]>()))
    }

    #[must_use]
    pub fn root_slot_secret(&self) -> SlotSecret {
        // this should always be there
        self.cached_tree[0][0].into()
    }

    #[must_use]
    pub fn current_slot(&self) -> Slot {
        self.starting_slot + self.current_index
    }

    #[must_use]
    pub const fn cache_depth(&self) -> usize {
        self.cached_tree.len()
    }

    #[must_use]
    pub fn merkle_path_for_index(&self, index: usize) -> Vec<Fr> {
        let mut cached_path = get_merkle_path(&self.cached_tree, index, self.cached_tree.len());
        // TODO: chose the proper tree depending the index
        let current_path = self.lower_sub_trees[0].merkle_path_for_index(index);
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
    pub fn merkle_path_for_index(&self, index: usize) -> Vec<Fr> {
        let hashed_leafs: Vec<_> =
            std::iter::successors(Some(self.seed), |seed| Some(ZkHasher::digest(&[*seed])))
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
) -> Vec<Fr> {
    let mut path = Vec::new();

    for level in (0..tree_depth).rev() {
        let sibling_index = if current_index.is_multiple_of(2) {
            current_index + 1
        } else {
            current_index - 1
        };

        if sibling_index < cached_tree[level].len() {
            path.push(cached_tree[level][sibling_index]);
        }

        current_index /= 2;
    }
    path.reverse();
    path
}

#[cfg(test)]
mod test {
    use groth16::fr_from_bytes;
    use nomos_core::crypto::{ZkDigest, ZkHasher};
    use rand::Rng as _;

    use crate::pol::merkle::MerklePolSubtree;
    //
    // #[test]
    // fn new_roots_are_consistent() {
    //     let merkle_pol =
    // MerklePolSubtree::new(fr_from_bytes(b"1987").unwrap(), 8);
    //     assert_eq!(merkle_pol.merkle_proof.len(), 9);
    //     let merkle_proof_root = merkle_pol
    //         .merkle_proof
    //         .iter()
    //         .copied()
    //         .reduce(|a, b| <ZkHasher as ZkDigest>::compress(&[a, b]))
    //         .unwrap();
    //     assert_eq!(merkle_proof_root, merkle_pol.slot_secret_root);
    // }
    //
    // #[test]
    // fn derive_next_proof() {
    //     let mut merkle_pol =
    // MerklePolSubtree::new(fr_from_bytes(b"1987").unwrap(), 8);
    //     let mut rng = rand::thread_rng();
    //     for _ in 0..rng.gen_range(2..250) {
    //         merkle_pol = merkle_pol.next_merkle();
    //     }
    //
    //     let mut merkle_proof_root = merkle_pol.merkle_proof[0];
    //     for i in 0..8 {
    //         if (u64::from(merkle_pol.current_index) >> i) & 1 == 1 {
    //             merkle_proof_root = <ZkHasher as ZkDigest>::compress(&[
    //                 merkle_pol.merkle_proof[i + 1],
    //                 merkle_proof_root,
    //             ]);
    //         } else {
    //             merkle_proof_root = <ZkHasher as ZkDigest>::compress(&[
    //                 merkle_proof_root,
    //                 merkle_pol.merkle_proof[i + 1],
    //             ]);
    //         }
    //     }
    //     assert_eq!(merkle_proof_root, merkle_pol.secret_root());
    // }
}
