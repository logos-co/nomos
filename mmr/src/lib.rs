use std::sync::OnceLock;

use digest::Digest;
use rpds::StackSync;

const EMPTY_VALUE: [u8; 32] = [0; 32];

/// An append-only persistent Merkle Mountain Range (MMR).
///
/// Compared to other merkle tree variants, this does not store leaves but
/// only the necessary internal nodes to update the root hash with new
/// additions. This makes it very space efficient, especially for large trees,
/// as we only need to store O(log n) nodes for n leaves.
///
/// Note on (de)serialization: serde will not preserve structural sharing since
/// it does not know which nodes are shared. This is ok if you only
/// (de)serialize one version of the tree, but if you dump multiple expect to
/// find multiple copes of the same nodes in the deserialized output. If you
/// need to preserve structural sharing, you should use a custom serialization.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct MerkleMountainRange<T, Hash, const MAX_HEIGHT: u8 = 32> {
    roots: StackSync<Root>,
    _hash: std::marker::PhantomData<(T, Hash)>,
}

impl<T, Hash, const MAX_HEIGHT: u8> PartialEq for MerkleMountainRange<T, Hash, MAX_HEIGHT> {
    fn eq(&self, other: &Self) -> bool {
        self.roots == other.roots
    }
}

impl<T, Hash, const MAX_HEIGHT: u8> Eq for MerkleMountainRange<T, Hash, MAX_HEIGHT> {}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Root {
    root: [u8; 32],
    height: u8,
}

impl<const MAX_HEIGHT: u8, T, Hash> Default for MerkleMountainRange<T, Hash, MAX_HEIGHT>
where
    T: AsRef<[u8]>,
    Hash: Digest<OutputSize = digest::typenum::U32>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<const MAX_HEIGHT: u8, T, Hash> MerkleMountainRange<T, Hash, MAX_HEIGHT>
where
    T: AsRef<[u8]>,
    Hash: Digest<OutputSize = digest::typenum::U32>,
{
    #[must_use]
    pub fn new() -> Self {
        assert!(
            MAX_HEIGHT <= 32,
            "MAX_HEIGHT must be less than or equal to 32"
        );
        Self {
            roots: StackSync::new_sync(),
            _hash: std::marker::PhantomData,
        }
    }

    #[must_use]
    pub fn push(&self, elem: T) -> Self {
        let root = {
            let mut hasher = Hash::new();
            hasher.update(elem.as_ref());
            hasher.finalize().into()
        };
        let mut last_root = Root { root, height: 1 };
        let mut roots = self.roots.clone();

        while let Some(root) = roots.peek().copied() {
            if last_root.height == root.height {
                roots.pop_mut();
                last_root = Root {
                    root: hash::<_, Hash>(root.root, last_root.root),
                    height: last_root.height + 1,
                };
                // we want the frontier root to have a fixed height, so each individual root
                // must be less than MAX_HEIGHT
                assert!(
                    last_root.height < MAX_HEIGHT,
                    "Height must be less than {MAX_HEIGHT}"
                );
            } else {
                break;
            }
        }

        roots = roots.push(last_root);

        Self {
            roots,
            _hash: std::marker::PhantomData,
        }
    }

    #[must_use]
    pub fn frontier_root(&self) -> [u8; 32] {
        let mut root = empty_subtree_root::<Hash>(0);
        let mut height = 0;
        for last in &self.roots {
            while height < last.height - 1 {
                root = hash::<_, Hash>(root, empty_subtree_root::<Hash>(height as usize));
                height += 1;
            }
            root = hash::<_, Hash>(last.root, root);
            height += 1;
        }
        assert!(height <= MAX_HEIGHT);
        // ensure a fixed depth
        while height < MAX_HEIGHT {
            root = hash::<_, Hash>(root, empty_subtree_root::<Hash>(height as usize));
            height += 1;
        }

        assert_eq!(height, MAX_HEIGHT);

        root
    }
}

fn hash<Item: AsRef<[u8]>, Hash: Digest<OutputSize = digest::typenum::U32>>(
    left: Item,
    right: Item,
) -> [u8; 32] {
    let mut hasher = Hash::new();
    hasher.update(left.as_ref());
    hasher.update(right.as_ref());
    hasher.finalize().into()
}

fn empty_subtree_root<Hash: Digest<OutputSize = digest::typenum::U32>>(height: usize) -> [u8; 32] {
    static PRECOMPUTED_EMPTY_ROOTS: OnceLock<[[u8; 32]; 32]> = OnceLock::new();
    assert!(height < 32, "Height must be less than 32: {height}");
    PRECOMPUTED_EMPTY_ROOTS.get_or_init(|| {
        let mut hashes = [EMPTY_VALUE; 32];
        for i in 1..32 {
            hashes[i] = hash::<[u8; 32], Hash>(hashes[i - 1], hashes[i - 1]);
        }
        hashes
    })[height]
}

#[cfg(test)]
mod test {
    use proptest_macro::property_test;
    type Blake2b = blake2::Blake2s256;
    use super::*;

    pub fn leaf(data: &[u8]) -> [u8; 32] {
        let mut hasher = Blake2b::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    #[test]
    fn test_empty_roots() {
        let mut root = [0; 32];
        for i in 0..32 {
            assert_eq!(root, empty_subtree_root::<Blake2b>(i));
            root = hash::<_, Blake2b>(root, root);
        }
    }

    fn padded_leaves(
        elements: impl IntoIterator<Item = impl AsRef<[u8]>>,
        height: u8,
    ) -> Vec<[u8; 32]> {
        let mut leaves = elements
            .into_iter()
            .map(|e| leaf(e.as_ref()))
            .collect::<Vec<_>>();
        let pad = (1 << height as usize) - leaves.len();
        leaves.extend(std::iter::repeat_n([0; 32], pad));
        leaves
    }

    fn root(elements: &[[u8; 32]]) -> [u8; 32] {
        let n = elements.len();
        assert!(n.is_power_of_two());
        let mut nodes = elements.to_vec();
        for h in (1..=n.ilog2()).rev() {
            for i in 0..2usize.pow(h - 1) {
                nodes[i] = hash::<_, Blake2b>(nodes[i * 2], nodes[i * 2 + 1]);
            }
        }

        nodes[0]
    }

    #[property_test]
    fn test_frontier_root_8(elems: Vec<[u8; 32]>) {
        let mut mmr = <MerkleMountainRange<_, Blake2b, 8>>::new();
        for elem in &elems {
            mmr = mmr.push(elem);
        }
        assert_eq!(mmr.frontier_root(), root(&padded_leaves(elems, 8)));
    }

    #[property_test]
    fn test_frontier_root_16(elems: Vec<[u8; 32]>) {
        let mut mmr = <MerkleMountainRange<_, Blake2b, 16>>::new();
        for elem in &elems {
            mmr = mmr.push(elem);
        }
        assert_eq!(mmr.frontier_root(), root(&padded_leaves(elems, 16)));
    }

    #[test]
    fn test_mmr_push() {
        let mut mmr = <MerkleMountainRange<_, Blake2b>>::new().push(b"hello".as_ref());

        assert_eq!(mmr.roots.size(), 1);
        assert_eq!(mmr.roots.peek().unwrap().height, 1);
        assert_eq!(mmr.roots.peek().unwrap().root, leaf(b"hello"));

        mmr = mmr.push(b"world".as_ref());

        assert_eq!(mmr.roots.size(), 1);
        assert_eq!(mmr.roots.peek().unwrap().height, 2);
        assert_eq!(
            mmr.roots.peek().unwrap().root,
            hash::<_, Blake2b>(leaf(b"hello"), leaf(b"world"))
        );

        mmr = mmr.push(b"!".as_ref());

        assert_eq!(mmr.roots.size(), 2);
        let top_root = mmr.roots.iter().last().unwrap();
        assert_eq!(top_root.height, 2);
        assert_eq!(
            top_root.root,
            hash::<_, Blake2b>(leaf(b"hello"), leaf(b"world"))
        );
        assert_eq!(mmr.roots.peek().unwrap().height, 1);
        assert_eq!(mmr.roots.peek().unwrap().root, leaf(b"!"));

        mmr = mmr.push(b"!".as_ref());

        assert_eq!(mmr.roots.size(), 1);
        assert_eq!(mmr.roots.peek().unwrap().height, 3);
        assert_eq!(
            mmr.roots.peek().unwrap().root,
            hash::<_, Blake2b>(
                hash::<_, Blake2b>(leaf(b"hello"), leaf(b"world")),
                hash::<_, Blake2b>(leaf(b"!"), leaf(b"!"))
            )
        );
    }
}
