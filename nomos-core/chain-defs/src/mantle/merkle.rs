use serde::{Deserialize, Serialize};

use crate::crypto::{Digest as _, Hasher};

#[must_use]
pub fn padded_leaves<const N: usize>(elements: &[Vec<u8>]) -> [[u8; 32]; N] {
    let mut leaves = [[0u8; 32]; N];

    for (i, element) in elements.iter().enumerate() {
        assert!(i < N);
        leaves[i] = leaf(element);
    }

    leaves
}

#[must_use]
pub fn leaf(data: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(b"NOMOS_MERKLE_LEAF");
    hasher.update(data);
    hasher.finalize().into()
}

#[must_use]
pub fn node(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(b"NOMOS_MERKLE_NODE");
    hasher.update(a);
    hasher.update(b);
    hasher.finalize().into()
}

#[must_use]
pub fn root<const N: usize>(elements: [[u8; 32]; N]) -> [u8; 32] {
    let n = elements.len();

    assert!(n.is_power_of_two());

    let mut nodes = elements;

    for h in (1..=n.ilog2()).rev() {
        for i in 0..2usize.pow(h - 1) {
            nodes[i] = node(nodes[i * 2], nodes[i * 2 + 1]);
        }
    }

    nodes[0]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathNode {
    Left([u8; 32]),
    Right([u8; 32]),
}

#[must_use]
pub fn path_root(leaf: [u8; 32], path: &[PathNode]) -> [u8; 32] {
    let mut computed_hash = leaf;

    for path_node in path {
        match path_node {
            PathNode::Left(sibling_hash) => {
                computed_hash = node(*sibling_hash, computed_hash);
            }
            PathNode::Right(sibling_hash) => {
                computed_hash = node(computed_hash, *sibling_hash);
            }
        }
    }

    computed_hash
}

#[must_use]
pub fn path<const N: usize>(leaves: [[u8; 32]; N], idx: usize) -> Vec<PathNode> {
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
            nodes[i] = node(nodes[i * 2], nodes[i * 2 + 1]);
        }
    }

    path
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_root_height_1() {
        let r = root::<1>(padded_leaves(&[b"sand".into()]));

        let expected = leaf(b"sand");

        assert_eq!(r, expected);
    }

    #[test]
    fn test_root_height_2() {
        let r = root::<2>(padded_leaves(&[b"desert".into(), b"sand".into()]));

        let expected = node(leaf(b"desert"), leaf(b"sand"));

        assert_eq!(r, expected);
    }

    #[test]
    fn test_root_height_3() {
        let r = root::<4>(padded_leaves(&[
            b"desert".into(),
            b"sand".into(),
            b"feels".into(),
            b"warm".into(),
        ]));

        let expected = node(
            node(leaf(b"desert"), leaf(b"sand")),
            node(leaf(b"feels"), leaf(b"warm")),
        );

        assert_eq!(r, expected);
    }

    #[test]
    fn test_root_height_4() {
        let r = root::<8>(padded_leaves(&[
            b"desert".into(),
            b"sand".into(),
            b"feels".into(),
            b"warm".into(),
            b"at".into(),
            b"night".into(),
        ]));

        let expected = node(
            node(
                node(leaf(b"desert"), leaf(b"sand")),
                node(leaf(b"feels"), leaf(b"warm")),
            ),
            node(
                node(leaf(b"at"), leaf(b"night")),
                node([0u8; 32], [0u8; 32]),
            ),
        );

        assert_eq!(r, expected);
    }

    #[test]
    fn test_path_height_1() {
        let leaves = padded_leaves(&[b"desert".into()]);
        let r = root::<1>(leaves);

        let p = path::<1>(leaves, 0);
        let expected = vec![];
        assert_eq!(p, expected);
        assert_eq!(path_root(leaf(b"desert"), &p), r);
    }

    #[test]
    fn test_path_height_2() {
        let leaves = padded_leaves(&[b"desert".into(), b"sand".into()]);
        let r = root::<2>(leaves);

        // --- proof for element at idx 0

        let p0 = path(leaves, 0);
        let expected0 = vec![PathNode::Right(leaf(b"sand"))];
        assert_eq!(p0, expected0);
        assert_eq!(path_root(leaf(b"desert"), &p0), r);

        // --- proof for element at idx 1

        let p1 = path(leaves, 1);
        let expected1 = vec![PathNode::Left(leaf(b"desert"))];
        assert_eq!(p1, expected1);
        assert_eq!(path_root(leaf(b"sand"), &p1), r);
    }

    #[test]
    fn test_path_height_3() {
        let leaves = padded_leaves(&[
            b"desert".into(),
            b"sand".into(),
            b"feels".into(),
            b"warm".into(),
        ]);
        let r = root::<4>(leaves);

        // --- proof for element at idx 0

        let p0 = path(leaves, 0);
        let expected0 = vec![
            PathNode::Right(leaf(b"sand")),
            PathNode::Right(node(leaf(b"feels"), leaf(b"warm"))),
        ];
        assert_eq!(p0, expected0);
        assert_eq!(path_root(leaf(b"desert"), &p0), r);

        // --- proof for element at idx 1

        let p1 = path(leaves, 1);
        let expected1 = vec![
            PathNode::Left(leaf(b"desert")),
            PathNode::Right(node(leaf(b"feels"), leaf(b"warm"))),
        ];
        assert_eq!(p1, expected1);
        assert_eq!(path_root(leaf(b"sand"), &p1), r);

        // --- proof for element at idx 2

        let p2 = path(leaves, 2);
        let expected2 = vec![
            PathNode::Right(leaf(b"warm")),
            PathNode::Left(node(leaf(b"desert"), leaf(b"sand"))),
        ];
        assert_eq!(p2, expected2);
        assert_eq!(path_root(leaf(b"feels"), &p2), r);

        // --- proof for element at idx 3

        let p3 = path(leaves, 3);
        let expected3 = vec![
            PathNode::Left(leaf(b"feels")),
            PathNode::Left(node(leaf(b"desert"), leaf(b"sand"))),
        ];
        assert_eq!(p3, expected3);
        assert_eq!(path_root(leaf(b"warm"), &p3), r);
    }
}
