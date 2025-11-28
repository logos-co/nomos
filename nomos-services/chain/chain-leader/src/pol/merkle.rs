use groth16::Fr;
use mmr::MerkleMountainRange;
use nomos_core::crypto::{ZkDigest, ZkHasher};

use crate::pol::SlotSecret;

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

        let mut mmr = MerkleMountainRange::<SlotSecret, ZkHasher>::new(tree_depth as u8);
        let mut merkle_proof: Vec<Fr> = Vec::new();

        for i in 1usize..=2usize.pow(tree_depth as u32) {
            let hash = hashed_leafs.next().unwrap();
            if i.is_power_of_two() {
                let mut proof_element = hash;
                if i != 1 {
                    let roots = mmr
                        .roots()
                        .iter()
                        .map(|root| root.root())
                        .take(mmr.roots().iter().count() - 1);

                    for root in roots {
                        proof_element = <ZkHasher as ZkDigest>::compress(&[root, proof_element]);
                    }
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

    pub fn next(&mut self) -> MerklePol {
        assert_ne!(
            self.current_index + 1,
            2usize.pow(self.tree_depth as u32) as u64
        );

        let index = self.current_index as usize;
        let depth_right_tree = 64 - ((index + 1) ^ index).leading_zeros() - 1;
        let right_tree = MerklePol::new(
            ZkHasher::digest(&[self.merkle_proof[0]]),
            depth_right_tree as usize,
        );

        let mut new_proof = right_tree.merkle_proof;
        let mut node = self.merkle_proof[0];
        for i in 0..self.tree_depth {
            if i >= (depth_right_tree as usize) {
                if i == depth_right_tree as usize {
                    new_proof.push(node);
                } else {
                    new_proof.push(self.merkle_proof[i + 1]);
                }
            } else {
                if (index >> i) & 1 == 1 {
                    node = <ZkHasher as ZkDigest>::compress(&[self.merkle_proof[i + 1], node]);
                } else {
                    node = <ZkHasher as ZkDigest>::compress(&[node, self.merkle_proof[i + 1]]);
                }
            }
        }

        MerklePol {
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
    use rand::Rng;

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
            merkle_pol = merkle_pol.next();
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

    #[test]
    fn power() {
        assert!(1usize.is_power_of_two())
    }
}
