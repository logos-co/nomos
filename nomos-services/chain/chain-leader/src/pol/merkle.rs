use cryptarchia_engine::Slot;
use futures::StreamExt;
use groth16::Fr;
use mmr::MerkleMountainRange;
use nomos_core::{
    crypto::{ZkDigest, ZkHasher},
};

use crate::pol::{MAX_TREE_DEPTH, SlotSecret};

pub struct MerklePol<const MAX_TREE_DEPTH: u8> {
    slot_secret_root: Fr,
    merkle_proof: Vec<Fr>,
    current_slot: Slot,
}

impl<const TREE_DEPTH: u8> MerklePol<TREE_DEPTH> {
    pub fn new(seed: Fr) -> Self {
        let mut hashed_leafs =
            std::iter::successors(Some(seed), |seed| Some(ZkHasher::digest(&[*seed])));

        let mut mmr = MerkleMountainRange::<SlotSecret, ZkHasher, TREE_DEPTH>::new();
        let mut merkle_proof: Vec<Fr> = Vec::new();

        for i in 1usize..=2usize.pow(TREE_DEPTH as u32) {
            let mut hash = hashed_leafs.next().unwrap();
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
            current_slot: Slot::new(0),
        }
    }


    pub fn next(&mut self) -> MerklePol<TREE_DEPTH> {
        assert_ne!(u64::from(self.current_slot)+1, (2usize.pow(TREE_DEPTH as u32)) as u64);

        let index = u64::from(self.current_slot) as usize;
        let depth_right_tree = 64 - ((index+1) ^ index).leading_zeros() - 1;
        let right_tree = MerklePol::<depth_right_tree>::new(self.merkle_proof[0]);

        let mut new_proof = right_tree.merkle_proof;
        let mut node = self.merkle_proof[0];
        for i in 0..(TREE_DEPTH as usize) {
            if i >= (depth_right_tree as usize) {
                new_proof.push(node);
            }
            if (index >> i) & 1 == 1 {
                node = <ZkHasher as ZkDigest>::compress(&[self.merkle_proof[i + 1], node]);
            } else {
                node = <ZkHasher as ZkDigest>::compress(&[node, self.merkle_proof[i + 1]]);
            }
        }

        MerklePol {
            slot_secret_root: self.slot_secret_root,
            merkle_proof: new_proof,
            current_slot: self.current_slot+1
        }
    }
}

#[cfg(test)]
mod test {
    use rand::Rng;
    use groth16::fr_from_bytes;
    use nomos_core::crypto::{ZkDigest, ZkHasher};

    use crate::pol::merkle::MerklePol;

    const MAX_TREE_DEPTH: u8 = 8;

    #[test]
    fn new_roots_are_consistent() {
        let merkle_pol = MerklePol::<MAX_TREE_DEPTH>::new(fr_from_bytes(b"1987").unwrap());
        assert_eq!(merkle_pol.merkle_proof.len(), (MAX_TREE_DEPTH + 1) as usize);
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
        let mut merkle_pol = MerklePol::<MAX_TREE_DEPTH>::new(fr_from_bytes(b"1987").unwrap());
        let mut rng = rand::thread_rng();
        for _ in 0..rng.gen_range(2..250) {
            merkle_pol = merkle_pol.next();
        }

        let mut merkle_proof_root = merkle_pol.merkle_proof[0];
        for i in 0..(MAX_TREE_DEPTH as usize) {
            if (u64::from(merkle_pol.current_slot) >> i) & 1 == 1 {
                merkle_proof_root = <ZkHasher as ZkDigest>::compress(&[merkle_pol.merkle_proof[i + 1], merkle_proof_root]);
            } else {
                merkle_proof_root = <ZkHasher as ZkDigest>::compress(&[merkle_proof_root, merkle_pol.merkle_proof[i + 1]]);
            }
        }
        assert_eq!(merkle_proof_root, merkle_pol.slot_secret_root);
    }

    #[test]
    fn power() {
        assert!(1usize.is_power_of_two())
    }
}
