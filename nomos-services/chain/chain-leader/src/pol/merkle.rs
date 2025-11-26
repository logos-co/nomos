use cryptarchia_engine::Slot;
use futures::StreamExt;
use groth16::Fr;
use mmr::MerkleMountainRange;
use nomos_core::{
    crypto::{ZkDigest, ZkHasher},
    utils::merkle::MerklePath,
};

use crate::pol::{MAX_TREE_DEPTH, SlotSecret};

pub type Mmr = MerkleMountainRange<SlotSecret, ZkHasher, { MAX_TREE_DEPTH + 1 }>;

pub struct MerklePol {
    slot_secret_root: Fr,
    merkle_proof: Vec<Fr>,
    current_slot: Slot,
}

impl MerklePol {
    pub fn new(seed: Fr) -> Self {
        let mut hashed_leafs =
            std::iter::successors(Some(seed), |seed| Some(ZkHasher::digest(&[*seed])));

        let mut mmr = Mmr::new();
        let mut merkle_proof: Vec<Fr> = Vec::new();

        for i in 1usize..=2usize.pow(MAX_TREE_DEPTH as u32) {
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

    // pub fn next(&mut self) -> SlotSecret {}
}

#[cfg(test)]
mod test {
    use groth16::fr_from_bytes;
    use nomos_core::crypto::{ZkDigest, ZkHasher};

    use crate::pol::{MAX_TREE_DEPTH, merkle::MerklePol};

    #[test]
    fn new_roots_are_consistent() {
        let merkle_pol = MerklePol::new(fr_from_bytes(b"1987").unwrap());
        assert_eq!(merkle_pol.merkle_proof.len(), MAX_TREE_DEPTH as usize + 1);
        let merkle_proof_root = merkle_pol
            .merkle_proof
            .iter()
            .take(MAX_TREE_DEPTH as usize + 1)
            .copied()
            .reduce(|a, b| <ZkHasher as ZkDigest>::compress(&[a, b]))
            .unwrap();
        assert_eq!(merkle_proof_root, merkle_pol.slot_secret_root);
    }

    #[test]
    fn power() {
        assert!(1usize.is_power_of_two())
    }
}
