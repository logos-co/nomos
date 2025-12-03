use groth16::Fr;
use mmr::{MerkleMountainRange, Root};
use nomos_core::crypto::{ZkDigest, ZkHasher};

use crate::pol::SlotSecret;

pub struct MerklePolCache {
    pub cached_tree: Vec<Vec<Fr>>,
    pub lower_merkle_pol: MerklePol,
    pub cache_depth: usize,
    pub tree_depth: usize,
}

impl MerklePolCache {
    pub fn new(seed: Fr, tree_depth: usize, cache_depth: usize) -> Self {
        let sub_tree_depth = tree_depth - cache_depth;
        // TODO: parallelize?
        let numer_of_trees = 2usize.pow(sub_tree_depth as u32);
        let total_leaves = 2usize.pow(tree_depth as u32);
        let seed_jump = total_leaves / numer_of_trees;
        let seeds: Vec<Fr> =
            std::iter::successors(Some(seed), |seed| Some(ZkHasher::digest(&[*seed]))).collect();
        let mut trees = seeds
            .chunks(seed_jump)
            .map(|leafs| MerklePol::from_leafs(leafs.iter().copied(), sub_tree_depth));
        let first_tree = trees.next().expect("At least one tree should be generated");
        let lower_tree_roots: Vec<Fr> = std::iter::once(first_tree.slot_secret_root.clone())
            .chain(trees.map(|merkle_pol| merkle_pol.slot_secret_root))
            .collect();
        let mut cached_tree: Vec<Vec<Fr>> =
            std::iter::successors(Some(lower_tree_roots), |roots| {
                if roots.len() <= 1 {
                    return None;
                }
                Some(
                    roots
                        .chunks(2)
                        .map(|pair| <[Fr; 2]>::try_from(pair).unwrap())
                        .map(|pair| <ZkHasher as ZkDigest>::compress(&pair))
                        .collect(),
                )
            })
            .collect();
        cached_tree.reverse();
        Self {
            cached_tree,
            lower_merkle_pol: first_tree,
            cache_depth,
            tree_depth,
        }
    }
}

pub struct MerklePol {
    slot_secret_root: Fr,
    merkle_proof: Vec<Fr>,
    current_index: u64,
    tree_depth: usize,
}

impl MerklePol {
    pub fn new(seed: Fr, tree_depth: usize) -> Self {
        let mut hashed_leafs =
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

    use crate::pol::merkle::MerklePol;

    #[test]
    fn new_roots_are_consistent() {
        let merkle_pol = MerklePol::new(fr_from_bytes(b"1987").unwrap(), 8);
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
        let mut merkle_pol = MerklePol::new(fr_from_bytes(b"1987").unwrap(), 8);
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
