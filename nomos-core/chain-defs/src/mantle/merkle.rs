use std::sync::LazyLock;

use groth16::{serde::serde_fr, Field as _, Fr};
use num_bigint::BigUint;
use poseidon2::{Digest, Poseidon2Bn254Hasher};
use serde::{Deserialize, Serialize};

use crate::mantle::NoteId;

#[must_use]
pub fn padded_leaves<const N: usize>(elements: &[NoteId]) -> [Fr; N] {
    let mut leaves = vec![Fr::ZERO; N];

    for (i, element) in elements.iter().enumerate() {
        assert!(i < N);
        leaves[i] = leaf(element.as_fr());
    }

    leaves.try_into().expect("Size is asserted per loop")
}

static NOMOS_MERKLE_LEAF: LazyLock<Fr> =
    LazyLock::new(|| BigUint::from_bytes_be(b"NOMOS_MERKLE_LEAF").into());

#[must_use]
pub fn leaf(data: &Fr) -> Fr {
    let mut hasher = Poseidon2Bn254Hasher::default();
    <Poseidon2Bn254Hasher as Digest>::update(&mut hasher, &NOMOS_MERKLE_LEAF);
    <Poseidon2Bn254Hasher as Digest>::update(&mut hasher, data);
    hasher.finalize()
}

static NOMOS_MERKLE_NODE: LazyLock<Fr> =
    LazyLock::new(|| BigUint::from_bytes_be(b"NOMOS_MERKLE_NODE").into());

#[must_use]
pub fn node(a: &Fr, b: &Fr) -> Fr {
    let mut hasher = Poseidon2Bn254Hasher::default();
    <Poseidon2Bn254Hasher as Digest>::update(&mut hasher, &NOMOS_MERKLE_NODE);
    <Poseidon2Bn254Hasher as Digest>::update(&mut hasher, a);
    <Poseidon2Bn254Hasher as Digest>::update(&mut hasher, b);
    hasher.finalize()
}

#[must_use]
pub fn root<const N: usize>(elements: [Fr; N]) -> Fr {
    let n = elements.len();

    assert!(n.is_power_of_two());

    let mut nodes = elements;

    for h in (1..=n.ilog2()).rev() {
        for i in 0..2usize.pow(h - 1) {
            nodes[i] = node(&nodes[i * 2], &nodes[i * 2 + 1]);
        }
    }

    nodes[0]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathNode {
    Left(#[serde(with = "serde_fr")] Fr),
    Right(#[serde(with = "serde_fr")] Fr),
}

#[must_use]
pub fn path_root(leaf: &Fr, path: &[PathNode]) -> Fr {
    let mut computed_hash = *leaf;

    for path_node in path {
        match path_node {
            PathNode::Left(sibling_hash) => {
                computed_hash = node(sibling_hash, &computed_hash);
            }
            PathNode::Right(sibling_hash) => {
                computed_hash = node(&computed_hash, sibling_hash);
            }
        }
    }

    computed_hash
}

#[must_use]
pub fn path<const N: usize>(leaves: [Fr; N], idx: usize) -> Vec<PathNode> {
    assert!(N.is_power_of_two());
    assert!(idx < N);

    let mut nodes = leaves;
    let mut path = Vec::new();
    let mut idx = idx;

    for h in (1..=N.ilog2()).rev() {
        if idx % 2 == 0 {
            path.push(PathNode::Right(nodes[idx + 1]));
        } else {
            path.push(PathNode::Left(nodes[idx - 1]));
        }

        idx /= 2;

        for i in 0..2usize.pow(h - 1) {
            nodes[i] = node(&nodes[i * 2], &nodes[i * 2 + 1]);
        }
    }

    path
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_root_height_1() {
        let sand: NoteId = Fr::from(BigUint::from_bytes_le(b"sand")).into();
        let r = root::<1>(padded_leaves(&[sand]));

        let expected = leaf(sand.as_fr());

        assert_eq!(r, expected);
    }

    #[test]
    fn test_root_height_2() {
        let desert: NoteId = Fr::from(BigUint::from_bytes_le(b"desert")).into();
        let sand: NoteId = Fr::from(BigUint::from_bytes_le(b"sand")).into();
        let r = root::<2>(padded_leaves(&[desert, sand]));

        let expected = node(&leaf(desert.as_fr()), &leaf(sand.as_fr()));

        assert_eq!(r, expected);
    }

    #[test]
    fn test_root_height_3() {
        let desert: NoteId = Fr::from(BigUint::from_bytes_le(b"desert")).into();
        let sand: NoteId = Fr::from(BigUint::from_bytes_le(b"sand")).into();
        let feels: NoteId = Fr::from(BigUint::from_bytes_le(b"feels")).into();
        let warm: NoteId = Fr::from(BigUint::from_bytes_le(b"warm")).into();
        let r = root::<4>(padded_leaves(&[desert, sand, feels, warm]));

        let expected = node(
            &node(&leaf(desert.as_fr()), &leaf(sand.as_fr())),
            &node(&leaf(feels.as_fr()), &leaf(warm.as_fr())),
        );

        assert_eq!(r, expected);
    }

    #[test]
    fn test_root_height_4() {
        let desert: NoteId = Fr::from(BigUint::from_bytes_le(b"desert")).into();
        let sand: NoteId = Fr::from(BigUint::from_bytes_le(b"sand")).into();
        let feels: NoteId = Fr::from(BigUint::from_bytes_le(b"feels")).into();
        let warm: NoteId = Fr::from(BigUint::from_bytes_le(b"warm")).into();
        let at: NoteId = Fr::from(BigUint::from_bytes_le(b"at")).into();
        let night: NoteId = Fr::from(BigUint::from_bytes_le(b"night")).into();

        let r = root::<8>(padded_leaves(&[desert, sand, feels, warm, at, night]));

        let expected = node(
            &node(
                &node(&leaf(desert.as_fr()), &leaf(sand.as_fr())),
                &node(&leaf(feels.as_fr()), &leaf(warm.as_fr())),
            ),
            &node(
                &node(&leaf(at.as_fr()), &leaf(night.as_fr())),
                &node(&Fr::from(BigUint::from(0u8)), &Fr::from(BigUint::from(0u8))),
            ),
        );

        assert_eq!(r, expected);
    }

    #[test]
    fn test_path_height_1() {
        let desert: NoteId = Fr::from(BigUint::from_bytes_le(b"desert")).into();
        let leaves = padded_leaves(&[desert]);
        let r = root::<1>(leaves);

        let p = path::<1>(leaves, 0);
        let expected = vec![];
        assert_eq!(p, expected);
        assert_eq!(path_root(&leaf(desert.as_fr()), &p), r);
    }

    #[test]
    fn test_path_height_2() {
        let desert: NoteId = Fr::from(BigUint::from_bytes_le(b"desert")).into();
        let sand: NoteId = Fr::from(BigUint::from_bytes_le(b"sand")).into();
        let leaves = padded_leaves(&[desert, sand]);
        let r = root::<2>(leaves);

        // --- proof for element at idx 0

        let p0 = path(leaves, 0);
        let expected0 = vec![PathNode::Right(leaf(sand.as_fr()))];
        assert_eq!(p0, expected0);
        assert_eq!(path_root(&leaf(desert.as_fr()), &p0), r);

        // --- proof for element at idx 1

        let p1 = path(leaves, 1);
        let expected1 = vec![PathNode::Left(leaf(desert.as_fr()))];
        assert_eq!(p1, expected1);
        assert_eq!(path_root(&leaf(sand.as_fr()), &p1), r);
    }

    #[test]
    fn test_path_height_3() {
        let desert: NoteId = Fr::from(BigUint::from_bytes_le(b"desert")).into();
        let sand: NoteId = Fr::from(BigUint::from_bytes_le(b"sand")).into();
        let feels: NoteId = Fr::from(BigUint::from_bytes_le(b"feels")).into();
        let warm: NoteId = Fr::from(BigUint::from_bytes_le(b"warm")).into();

        let leaves = padded_leaves(&[desert, sand, feels, warm]);
        let r = root::<4>(leaves);

        // --- proof for element at idx 0

        let p0 = path(leaves, 0);
        let expected0 = vec![
            PathNode::Right(leaf(sand.as_fr())),
            PathNode::Right(node(&leaf(feels.as_fr()), &leaf(warm.as_fr()))),
        ];
        assert_eq!(p0, expected0);
        assert_eq!(path_root(&leaf(desert.as_fr()), &p0), r);

        // --- proof for element at idx 1

        let p1 = path(leaves, 1);
        let expected1 = vec![
            PathNode::Left(leaf(desert.as_fr())),
            PathNode::Right(node(&leaf(feels.as_fr()), &leaf(warm.as_fr()))),
        ];
        assert_eq!(p1, expected1);
        assert_eq!(path_root(&leaf(sand.as_fr()), &p1), r);

        // --- proof for element at idx 2

        let p2 = path(leaves, 2);
        let expected2 = vec![
            PathNode::Right(leaf(warm.as_fr())),
            PathNode::Left(node(&leaf(desert.as_fr()), &leaf(sand.as_fr()))),
        ];
        assert_eq!(p2, expected2);
        assert_eq!(path_root(&leaf(feels.as_fr()), &p2), r);

        // --- proof for element at idx 3

        let p3 = path(leaves, 3);
        let expected3 = vec![
            PathNode::Left(leaf(feels.as_fr())),
            PathNode::Left(node(&leaf(desert.as_fr()), &leaf(sand.as_fr()))),
        ];
        assert_eq!(p3, expected3);
        assert_eq!(path_root(&leaf(warm.as_fr()), &p3), r);
    }
}
