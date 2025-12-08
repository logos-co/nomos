use groth16::Fr;
use mmr::{MerkleMountainRange, Root};
use nomos_core::crypto::{ZkDigest, ZkHasher};
use rayon::iter::{IntoParallelIterator as _, ParallelExtend as _, ParallelIterator as _};

use crate::pol::SlotSecret;

pub type CachedTree = Vec<Vec<Fr>>;

pub struct MerklePolCache {
    pub cached_tree: CachedTree,
    pub lower_merkle_pol: MerklePolSubtree,
    pub cache_depth: usize,
    pub tree_depth: usize,
    pub current_index: usize,
}

impl MerklePolCache {
    #[must_use]
    pub fn new(seed: Fr, tree_depth: usize, cache_depth: usize) -> Self {
        let sub_tree_depth = tree_depth - cache_depth;
        let numer_of_trees = 2usize.pow(cache_depth as u32);
        let total_leaves = 2usize.pow(tree_depth as u32);
        let seed_jump = total_leaves / numer_of_trees;
        // we have to pre-cache the leaves so we can parallelize the subtree computation
        let seeds: Vec<Fr> = Self::leaves_from_seed(seed).take(total_leaves).collect();
        // we just recompute the first tree because after we have computed the root
        // secret, we will compute a subtree at a time for each slot secret path
        let first_tree =
            MerklePolSubtree::from_leafs(seeds.iter().take(seed_jump).copied(), sub_tree_depth);
        // to parallelize the computation of the lower tree roots,
        // we need to pre-cache the slices of the leaves to use par-iter
        let seeds_chunks: Vec<&[Fr]> = seeds.chunks(seed_jump).skip(1).collect();
        let mut lower_tree_roots = Vec::with_capacity(numer_of_trees);
        lower_tree_roots.push(first_tree.slot_secret_root);
        lower_tree_roots.par_extend(seeds_chunks.into_par_iter().map(|leafs| {
            MerklePolSubtree::from_leafs(leafs.iter().copied(), sub_tree_depth).slot_secret_root
        }));
        lower_tree_roots.reverse();

        // compute the root slot secret
        let cached_tree = Self::compute_cached_tree_from_leafs(lower_tree_roots);

        Self {
            cached_tree,
            lower_merkle_pol: first_tree,
            cache_depth,
            tree_depth,
            current_index: 0,
        }
    }

    pub fn leaves_from_seed(seed: Fr) -> impl Iterator<Item = Fr> {
        std::iter::successors(Some(seed), |seed| Some(ZkHasher::digest(&[*seed])))
    }

    #[must_use]
    pub fn root_slot_secret(&self) -> Fr {
        // this should always be there
        self.cached_tree[0][0]
    }

    fn compute_cached_tree_from_leafs(tree_leafs: Vec<Fr>) -> Vec<Vec<Fr>> {
        let mut cached_tree: Vec<Vec<Fr>> = std::iter::successors(Some(tree_leafs), |leafs| {
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

    pub fn next_slot(&mut self) {
        self.current_index += 1;
        self.lower_merkle_pol = self.lower_merkle_pol.next_merkle();
    }

    #[must_use]
    pub fn get_merkle_path(&self) -> Vec<Fr> {
        let mut path = Vec::new();
        let mut current_index = self.current_index;

        for level in (0..self.tree_depth).rev() {
            let sibling_index = if current_index.is_multiple_of(2) {
                current_index + 1
            } else {
                current_index - 1
            };

            if sibling_index < self.cached_tree[level].len() {
                path.push(self.cached_tree[level][sibling_index]);
            }

            current_index /= 2;
        }
        path.reverse();
        path.extend(self.lower_merkle_pol.merkle_proof.iter().copied());
        path
    }
}

pub struct MerklePolSubtree {
    slot_secret_root: Fr,
    merkle_proof: Vec<Fr>,
    current_index: u64,
    tree_depth: usize,
}

impl MerklePolSubtree {
    #[must_use]
    pub fn new(seed: Fr, tree_depth: usize) -> Self {
        let hashed_leafs =
            std::iter::successors(Some(seed), |seed| Some(ZkHasher::digest(&[*seed])));
        Self::from_leafs(hashed_leafs, tree_depth)
    }

    pub fn from_leafs(mut leafs: impl Iterator<Item = Fr>, tree_depth: usize) -> Self {
        let mut mmr = MerkleMountainRange::<SlotSecret, ZkHasher>::new(tree_depth as u8);
        let mut merkle_proof: Vec<Fr> = Vec::new();

        for i in 1usize..=2usize.pow(tree_depth as u32) {
            let hash = leafs.next().unwrap();
            if i.is_power_of_two() {
                let mut proof_element = hash;
                if i != 1 {
                    let roots = mmr
                        .roots()
                        .iter()
                        .map(Root::root)
                        .take(mmr.roots().iter().count() - 1);
                    proof_element = roots.fold(proof_element, |acc, root| {
                        <ZkHasher as ZkDigest>::compress(&[root, acc])
                    });
                }
                merkle_proof.push(proof_element);
            }
            mmr = mmr.push(hash.into());
        }

        let slot_secret_root = mmr.roots().peek().unwrap().root();

        Self {
            slot_secret_root,
            merkle_proof,
            current_index: 0,
            tree_depth,
        }
    }

    #[must_use]
    pub fn next_merkle(&mut self) -> Self {
        assert_ne!(
            self.current_index + 1,
            2usize.pow(self.tree_depth as u32) as u64
        );

        let index = self.current_index as usize;
        let depth_right_tree = 64 - ((index + 1) ^ index).leading_zeros() - 1;
        let right_tree = Self::new(
            ZkHasher::digest(&[self.merkle_proof[0]]),
            depth_right_tree as usize,
        );

        let mut new_proof = right_tree.merkle_proof;
        let mut node = self.merkle_proof[0];
        for i in 0..self.tree_depth {
            if i == depth_right_tree as usize {
                new_proof.push(node);
            } else if i > (depth_right_tree as usize) {
                new_proof.push(self.merkle_proof[i + 1]);
            } else if (index >> i) & 1 == 1 {
                node = <ZkHasher as ZkDigest>::compress(&[self.merkle_proof[i + 1], node]);
            } else {
                node = <ZkHasher as ZkDigest>::compress(&[node, self.merkle_proof[i + 1]]);
            }
        }

        Self {
            slot_secret_root: self.slot_secret_root,
            merkle_proof: new_proof,
            current_index: self.current_index + 1,
            tree_depth: self.tree_depth,
        }
    }
}

#[cfg(test)]
mod test {
    use groth16::fr_from_bytes;
    use nomos_core::crypto::{ZkDigest, ZkHasher};
    use rand::Rng as _;

    use crate::pol::merkle::MerklePolSubtree;

    #[test]
    fn new_roots_are_consistent() {
        let merkle_pol = MerklePolSubtree::new(fr_from_bytes(b"1987").unwrap(), 8);
        assert_eq!(merkle_pol.merkle_proof.len(), 9);
        let merkle_proof_root = merkle_pol
            .merkle_proof
            .iter()
            .copied()
            .reduce(|a, b| <ZkHasher as ZkDigest>::compress(&[a, b]))
            .unwrap();
        assert_eq!(merkle_proof_root, merkle_pol.slot_secret_root);
    }

    #[test]
    fn derive_next_proof() {
        let mut merkle_pol = MerklePolSubtree::new(fr_from_bytes(b"1987").unwrap(), 8);
        let mut rng = rand::thread_rng();
        for _ in 0..rng.gen_range(2..250) {
            merkle_pol = merkle_pol.next_merkle();
        }

        let mut merkle_proof_root = merkle_pol.merkle_proof[0];
        for i in 0..8 {
            if (u64::from(merkle_pol.current_index) >> i) & 1 == 1 {
                merkle_proof_root = <ZkHasher as ZkDigest>::compress(&[
                    merkle_pol.merkle_proof[i + 1],
                    merkle_proof_root,
                ]);
            } else {
                merkle_proof_root = <ZkHasher as ZkDigest>::compress(&[
                    merkle_proof_root,
                    merkle_pol.merkle_proof[i + 1],
                ]);
            }
        }
        assert_eq!(merkle_proof_root, merkle_pol.slot_secret_root);
    }
}
