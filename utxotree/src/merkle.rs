use std::{
    marker::PhantomData,
    sync::{Arc, OnceLock},
};

use digest::Digest;
use rpds::Queue;

const EMPTY_VALUE: [u8; 32] = [0; 32];

fn empty_subtree_root<Hash: Digest<OutputSize = digest::typenum::U32>>(height: usize) -> [u8; 32] {
    static PRECOMPUTED_EMPTY_ROOTS: OnceLock<[[u8; 32]; 32]> = OnceLock::new();
    assert!(height < 32, "Height must be less than 32: {height}");
    PRECOMPUTED_EMPTY_ROOTS.get_or_init(|| {
        let mut hashes = [EMPTY_VALUE; 32];
        for i in 1..32 {
            let mut hasher = Hash::new();
            hasher.update(hashes[i - 1]);
            hasher.update(hashes[i - 1]);
            hashes[i] = hasher.finalize().into();
        }
        hashes
    })[height]
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node<Item> {
    Inner {
        left: Arc<Node<Item>>,
        right: Arc<Node<Item>>,
        value: [u8; 32],
        right_subtree_size: usize,
        left_subtree_size: usize,
        height: usize,
    },
    // An empty inner node, representing an unexpanded empty subtree, to avoid
    // allocating a full subtree when not necessary.
    // Can only be found in the right subtree of an inner node.
    Empty {
        height: usize,
    },
    // A leaf node (possibly) containing an item, will be empty after a removal
    Leaf {
        item: Option<Item>,
    },
}

fn hash<Item: AsRef<[u8]>, Hash: Digest<OutputSize = digest::typenum::U32>>(
    left: &Node<Item>,
    right: &Node<Item>,
) -> [u8; 32] {
    let mut hasher = Hash::new();
    match left {
        Node::Inner { value, .. } => hasher.update(value),
        Node::Leaf { item } => {
            hasher.update(item.as_ref().map_or(EMPTY_VALUE.as_ref(), AsRef::as_ref));
        }
        Node::Empty { .. } => panic!("Empty node in left subtree is not allowed"),
    }
    match right {
        Node::Inner { value, .. } => hasher.update(value),
        Node::Leaf { item } => {
            hasher.update(item.as_ref().map_or(EMPTY_VALUE.as_ref(), AsRef::as_ref));
        }
        Node::Empty { height } => {
            hasher.update(empty_subtree_root::<Hash>(*height));
        }
    }
    hasher.finalize().into()
}

impl<Item> Node<Item> {
    const fn new(item: Item) -> Self {
        Self::Leaf { item: Some(item) }
    }

    fn size(&self) -> usize {
        match self {
            Self::Inner {
                left_subtree_size,
                right_subtree_size,
                ..
            } => left_subtree_size + right_subtree_size,
            Self::Leaf { item: Some(_) } => 1,
            Self::Empty { .. } | Self::Leaf { item: None } => 0,
        }
    }

    // size of the full subtree
    const fn capacity(&self) -> usize {
        1 << self.height()
    }

    const fn height(&self) -> usize {
        match self {
            Self::Inner { height, .. } | Self::Empty { height } => *height,
            Self::Leaf { .. } => 0,
        }
    }
}

impl<Item: AsRef<[u8]>> Node<Item> {
    fn new_inner<Hash>(left: Arc<Self>, right: Arc<Self>) -> Self
    where
        Hash: Digest<OutputSize = digest::typenum::U32>,
    {
        Self::Inner {
            right_subtree_size: right.size(),
            left_subtree_size: left.size(),
            height: left.height().max(right.height()) + 1,
            value: hash::<_, Hash>(&left, &right),
            left,
            right,
        }
    }

    fn insert_or_modify<Hash, F: FnOnce(&Self) -> Self>(
        self: &Arc<Self>,
        index: usize,
        f: F,
    ) -> Arc<Self>
    where
        Hash: Digest<OutputSize = digest::typenum::U32>,
    {
        match self.as_ref() {
            Self::Inner { left, right, .. } => {
                assert!(
                    index < self.capacity(),
                    "Index {} out of bounds for inner node with height {}",
                    index,
                    self.height()
                );

                if index < left.capacity() {
                    // modify the left subtree
                    Arc::new(Self::new_inner::<Hash>(
                        left.insert_or_modify::<Hash, _>(index, f),
                        Arc::clone(right),
                    ))
                } else {
                    // modify the right subtree
                    Arc::new(Self::new_inner::<Hash>(
                        Arc::clone(left),
                        right.insert_or_modify::<Hash, _>(index - left.capacity(), f),
                    ))
                }
            }
            Self::Empty { height } if *height > 0 => {
                // expand the empty subtree to modify the new item
                assert!(
                    index == 0,
                    "Cannot expand an empty subtree more than one node at a time",
                );
                Arc::new(Self::new_inner::<Hash>(
                    Arc::new(Self::Empty { height: height - 1 })
                        .insert_or_modify::<Hash, _>(index, f),
                    Arc::new(Self::Empty { height: height - 1 }),
                ))
            }
            Self::Leaf { .. } | Self::Empty { .. } => {
                assert!(
                    index == 0,
                    "Cannot insert into a terminal node with index !=0",
                );
                Arc::new(f(self))
            }
        }
    }

    fn insert_at<Hash>(self: &Arc<Self>, index: usize, item: Item) -> Arc<Self>
    where
        Hash: Digest<OutputSize = digest::typenum::U32>,
    {
        self.insert_or_modify::<Hash, _>(index, |node| match node {
            Self::Leaf { item: None } | Self::Empty { .. } => Self::new(item),
            Self::Leaf { item: Some(_) } => panic!("Cannot insert into a non-empty leaf node"),
            _ => panic!("Cannot insert into a non-terminal node"),
        })
    }

    fn remove_at<Hash>(self: &Arc<Self>, index: usize) -> Arc<Self>
    where
        Hash: Digest<OutputSize = digest::typenum::U32>,
    {
        self.insert_or_modify::<Hash, _>(index, move |node| match node {
            Self::Leaf { item: Some(_) } => Self::Leaf { item: None },
            _ => panic!("Cannot remove from a empty / non-leaf node"),
        })
    }
}

/// A dynamic persistent Merkle tree that supports insertion and removal of
/// items. Removed items are replaced with an empty leaf node, which prevents
/// the whole tree reordering and their position is recorded for future
/// insertions. Compared to a MPT, the height of this tree is predictable and
/// bounded by the number of items, allowing for efficient and simple proof of
/// memberships for Proof of Leadership.
#[derive(Debug, Clone)]
pub struct DynamicMerkleTree<Item, Hash> {
    root: Arc<Node<Item>>,
    holes: Queue<usize>,
    _hash: PhantomData<Hash>,
}

impl<Item: AsRef<[u8]>, Hash: Digest<OutputSize = digest::typenum::U32>>
    DynamicMerkleTree<Item, Hash>
{
    pub fn new() -> Self {
        let holes = Queue::new().enqueue(0);
        Self {
            root: Arc::new(Node::Empty { height: 31 }),
            holes,
            _hash: PhantomData,
        }
    }

    pub fn size(&self) -> usize {
        self.root.size()
    }

    pub(crate) fn insert(&self, item: Item) -> (Self, usize) {
        assert!(
            self.size() < self.root.capacity(),
            "max capacity reached, cannot insert more items"
        );

        let (holes, index) = self.holes.peek().map_or_else(
            || (self.holes.clone(), self.root.size()),
            |hole| (self.holes.dequeue().unwrap(), *hole),
        );

        let root = self.root.insert_at::<Hash>(index, item);
        (
            Self {
                root,
                holes,
                _hash: PhantomData,
            },
            index,
        )
    }

    pub(crate) fn remove(&self, index: usize) -> Self {
        assert!(index < self.root.capacity(), "Index out of bounds");

        let root = self.root.remove_at::<Hash>(index);
        let holes = self.holes.enqueue(index);
        Self {
            root,
            holes,
            _hash: PhantomData,
        }
    }

    pub fn root(&self) -> [u8; 32] {
        match self.root.as_ref() {
            Node::Inner { value, .. } => *value,
            Node::Leaf { .. } => {
                panic!("Cannot get root from a leaf node, expected an inner node or empty node");
            }
            Node::Empty { .. } => empty_subtree_root::<Hash>(self.root.height()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestHash = blake2::Blake2b<digest::typenum::U32>;

    #[test]
    fn test_empty_tree() {
        let tree: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();
        assert_eq!(tree.size(), 0);
        assert_eq!(tree.root(), empty_subtree_root::<TestHash>(31));
    }

    #[test]
    fn test_single_insert() {
        let tree: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();
        let item = b"test".to_vec();
        let (tree_with_item, index) = tree.insert(item);

        assert_eq!(tree_with_item.size(), 1);
        assert_eq!(index, 0);
        assert_ne!(tree_with_item.root(), tree.root());
        assert!(matches!(tree_with_item.root.as_ref(), &Node::Inner { .. }));
    }

    #[test]
    fn test_multiple_inserts() {
        let mut tree: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();
        let items = [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];

        for (i, item) in items.iter().enumerate() {
            let (new_tree, index) = tree.insert(item.clone());
            tree = new_tree;
            assert_eq!(tree.size(), i + 1);
            assert_eq!(index, i);
        }

        assert_eq!(tree.size(), 3);
    }

    #[test]
    fn test_remove_single_item() {
        let tree: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();
        let item = b"test".to_vec();
        let (tree_with_item, _) = tree.insert(item);

        let tree_after_removal = tree_with_item.remove(0);
        assert_eq!(tree_after_removal.size(), 0);
        assert_eq!(tree_after_removal.root(), tree.root());
    }

    #[test]
    fn test_remove_and_reinsert() {
        let mut tree: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();
        let items = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];

        for item in &items {
            let (new_tree, _) = tree.insert(item.clone());
            tree = new_tree;
        }

        let tree_after_removal = tree.remove(1);
        assert_eq!(tree_after_removal.size(), 2);

        let (tree_after_reinsert, index) = tree_after_removal.insert(b"d".to_vec());
        assert_eq!(tree_after_reinsert.size(), 3);
        assert_eq!(index, 1);
    }

    #[test]
    fn test_hole_management() {
        let tree: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();

        let (tree1, _) = tree.insert(b"a".to_vec());
        let (tree2, _) = tree1.insert(b"b".to_vec());
        let (tree3, _) = tree2.insert(b"c".to_vec());

        let tree_removed = tree3.remove(1);
        assert_eq!(tree_removed.size(), 2);

        let (tree_reinserted, index) = tree_removed.insert(b"d".to_vec());
        assert_eq!(index, 1);
        assert_eq!(tree_reinserted.size(), 3);
    }

    #[test]
    fn test_root_consistency() {
        let tree: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();
        let (tree1, _) = tree.insert(b"a".to_vec());
        let (tree2, _) = tree1.insert(b"b".to_vec());

        let root1 = tree2.root();

        let tree_removed = tree2.remove(0);
        let (tree_reinserted, _) = tree_removed.insert(b"a".to_vec());
        let root2 = tree_reinserted.root();

        assert_eq!(root1, root2);
    }

    #[test]
    fn test_deterministic_root() {
        let tree1: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();
        let (tree1, _) = tree1.insert(b"a".to_vec());
        let (tree1, _) = tree1.insert(b"b".to_vec());

        let tree2: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();
        let (tree2, _) = tree2.insert(b"a".to_vec());
        let (tree2, _) = tree2.insert(b"b".to_vec());

        assert_eq!(tree1.root(), tree2.root());
    }

    #[test]
    #[should_panic(expected = "Index out of bounds")]
    fn test_remove_out_of_bounds() {
        let tree: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();
        let (tree, _) = tree.insert(b"test".to_vec());
        tree.remove(1 << 32);
    }

    #[test]
    fn test_structural_sharing() {
        let tree1: DynamicMerkleTree<Vec<u8>, TestHash> = DynamicMerkleTree::new();
        let (tree2, _) = tree1.insert(b"a".to_vec());
        let (tree3, _) = tree2.insert(b"b".to_vec());

        assert_eq!(tree1.size(), 0);
        assert_eq!(tree2.size(), 1);
        assert_eq!(tree3.size(), 2);

        let tree4 = tree2.remove(0);
        assert_eq!(tree4.size(), 0);
        assert_eq!(tree2.size(), 1);
    }
}
