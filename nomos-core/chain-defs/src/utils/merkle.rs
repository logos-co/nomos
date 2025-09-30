use groth16::Fr;
use poseidon2::Digest as _;

use crate::crypto::ZkHasher;

#[derive(Clone)]
pub enum MerkleNode<T> {
    Left(T),
    Right(T),
}

impl<T> MerkleNode<T> {
    pub const fn item(&self) -> &T {
        match self {
            Self::Left(v) | Self::Right(v) => v,
        }
    }
}

pub type MerklePath<T> = Vec<MerkleNode<T>>;

pub fn calculate_merkle_root<T>(leaves: &[MerkleNode<T>]) -> Option<Fr>
where
    T: Into<Fr> + Copy,
{
    if leaves.is_empty() {
        return None;
    }

    let mut current_level: Vec<Fr> = leaves.iter().map(|node| (*node.item()).into()).collect();

    while current_level.len() > 1 {
        let mut next_level = Vec::new();

        for chunk in current_level.chunks(2) {
            let combined = if chunk.len() == 2 {
                ZkHasher::digest(&[chunk[0], chunk[1]])
            } else {
                chunk[0]
            };
            next_level.push(combined);
        }

        current_level = next_level;
    }

    Some(current_level[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_none() {
        let leaves: Vec<MerkleNode<Fr>> = vec![];
        assert_eq!(calculate_merkle_root(&leaves), None);
    }

    #[test]
    fn single_leaf_returns_unchanged() {
        let leaves = vec![MerkleNode::Left(Fr::from(42u64))];
        assert_eq!(calculate_merkle_root(&leaves), Some(Fr::from(42u64)));
    }

    #[test]
    fn two_leaves_hashed() {
        let leaves = vec![
            MerkleNode::Left(Fr::from(1u64)),
            MerkleNode::Right(Fr::from(2u64)),
        ];

        let root = calculate_merkle_root(&leaves);
        let expected = ZkHasher::digest(&[Fr::from(1u64), Fr::from(2u64)]);

        assert_eq!(root, Some(expected));
    }

    #[test]
    fn three_leaves() {
        let leaves = vec![
            MerkleNode::Left(Fr::from(1u64)),
            MerkleNode::Right(Fr::from(2u64)),
            MerkleNode::Left(Fr::from(3u64)),
        ];
        let root = calculate_merkle_root(&leaves);

        let level1_hash = ZkHasher::digest(&[Fr::from(1u64), Fr::from(2u64)]);
        let expected = ZkHasher::digest(&[level1_hash, Fr::from(3u64)]);

        assert_eq!(root, Some(expected));
    }

    #[test]
    fn four_leaves() {
        let leaves = vec![
            MerkleNode::Left(Fr::from(1u64)),
            MerkleNode::Right(Fr::from(2u64)),
            MerkleNode::Left(Fr::from(3u64)),
            MerkleNode::Right(Fr::from(4u64)),
        ];
        let root = calculate_merkle_root(&leaves);

        let left_hash = ZkHasher::digest(&[Fr::from(1u64), Fr::from(2u64)]);
        let right_hash = ZkHasher::digest(&[Fr::from(3u64), Fr::from(4u64)]);
        let expected = ZkHasher::digest(&[left_hash, right_hash]);

        assert_eq!(root, Some(expected));
    }

    #[test]
    fn five_leaves() {
        let leaves = vec![
            MerkleNode::Left(Fr::from(1u64)),
            MerkleNode::Right(Fr::from(2u64)),
            MerkleNode::Left(Fr::from(3u64)),
            MerkleNode::Right(Fr::from(4u64)),
            MerkleNode::Left(Fr::from(5u64)),
        ];
        let root = calculate_merkle_root(&leaves);

        let hash1 = ZkHasher::digest(&[Fr::from(1u64), Fr::from(2u64)]);
        let hash2 = ZkHasher::digest(&[Fr::from(3u64), Fr::from(4u64)]);
        let level2_left = ZkHasher::digest(&[hash1, hash2]);
        let expected = ZkHasher::digest(&[level2_left, Fr::from(5u64)]);

        assert_eq!(root, Some(expected));
    }
}
