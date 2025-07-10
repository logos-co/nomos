mod merkle;

use std::{collections::BTreeMap, marker::PhantomData};

use digest::Digest;
use merkle::DynamicMerkleTree;
use rpds::HashTrieMapSync;
use thiserror::Error;

/// A store for `UTxOs` that allows for efficient insertion, removal, and
/// retrieval of items, while efficiently maintaining a compact Merkle tree
/// for Proof of Leadership (`PoL`) generation.
///
/// Note on (de)serialization: serde will not preserve structural sharing since
/// it does not know which nodes are shared. This is ok if you only
/// (de)serialize one version of the tree, but if you dump multiple expect to
/// find multiple copes of the same nodes in the deserialized output. If you
/// need to preserve structural sharing, you should use a custom serialization.
#[derive(Debug, Clone)]
pub struct UtxoTree<Key, Item, F, Hash>
where
    Key: std::hash::Hash + Eq,
{
    merkle: DynamicMerkleTree<Key, Hash>,
    // key -> (item, position in merkle tree)
    items: HashTrieMapSync<Key, (Item, usize)>,
    item_to_key: PhantomData<F>,
}

pub trait KeyExtractor<Item, Key> {
    fn extract(item: &Item) -> Key;
}

impl<Key, Item, F, Hash> Default for UtxoTree<Key, Item, F, Hash>
where
    Key: AsRef<[u8]> + Clone + std::hash::Hash + Eq,
    Hash: Digest<OutputSize = digest::typenum::U32>,
    F: KeyExtractor<Item, Key>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<Key, Item, F, Hash> UtxoTree<Key, Item, F, Hash>
where
    Key: AsRef<[u8]> + Clone + std::hash::Hash + Eq,
    Hash: Digest<OutputSize = digest::typenum::U32>,
    F: KeyExtractor<Item, Key>,
{
    #[must_use]
    pub fn new() -> Self {
        Self {
            merkle: DynamicMerkleTree::new(),
            items: HashTrieMapSync::new_sync(),
            item_to_key: PhantomData,
        }
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.merkle.size()
    }
}

#[derive(Error, Debug, Clone)]
pub enum Error {
    #[error("Item not found")]
    NotFound,
}

impl<Key, Item, F, Hash> UtxoTree<Key, Item, F, Hash>
where
    Key: AsRef<[u8]> + Clone + std::hash::Hash + Eq,
    Hash: Digest<OutputSize = digest::typenum::U32>,
    Item: Clone,
    F: KeyExtractor<Item, Key>,
{
    pub fn insert(&self, item: Item) -> (Self, usize) {
        let key = <F as KeyExtractor<_, _>>::extract(&item);
        let (merkle, pos) = self.merkle.insert(key.clone());
        let items = self.items.insert(key, (item, pos));
        (
            Self {
                merkle,
                items,
                item_to_key: PhantomData,
            },
            pos,
        )
    }

    pub fn contains(&self, item: &Item) -> bool {
        let key = <F as KeyExtractor<_, _>>::extract(item);
        self.items.contains_key(&key)
    }

    #[must_use]
    pub const fn utxos(&self) -> &HashTrieMapSync<Key, (Item, usize)> {
        &self.items
    }

    pub fn remove(&self, key: &Key) -> Result<(Self, Item), Error> {
        let Some((item, pos)) = self.items.get(key) else {
            return Err(Error::NotFound);
        };
        let items = self.items.remove(key);
        let merkle = self.merkle.remove(*pos);

        Ok((
            Self {
                items,
                merkle,
                item_to_key: self.item_to_key,
            },
            item.clone(),
        ))
    }

    #[must_use]
    pub fn root(&self) -> [u8; 32] {
        self.merkle.root()
    }

    pub fn witness(&self, key: &Key) -> Option<()> {
        self.items.contains_key(key).then_some(())
    }

    #[must_use] pub fn compressed(&self) -> CompressedUtxoTree<Item>
    where
        Item: Clone,
    {
        CompressedUtxoTree {
            items: self
                .items
                .iter()
                .map(|(_k, (v, pos))| (*pos, v.clone()))
                .collect(),
        }
    }
}

impl<Key, Item, F, Hash> PartialEq for UtxoTree<Key, Item, F, Hash>
where
    Key: AsRef<[u8]> + std::hash::Hash + Eq,
    Item: PartialEq,
    Hash: Digest<OutputSize = digest::typenum::U32>,
{
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items && self.merkle == other.merkle
    }
}

impl<Key, Item, F, Hash> Eq for UtxoTree<Key, Item, F, Hash>
where
    Key: AsRef<[u8]> + std::hash::Hash + Eq,
    Item: Eq,
    Hash: Digest<OutputSize = digest::typenum::U32>,
{
}

impl<Key, Item, F, Hash> FromIterator<Item> for UtxoTree<Key, Item, F, Hash>
where
    Key: AsRef<[u8]> + Clone + std::hash::Hash + Eq,
    Hash: Digest<OutputSize = digest::typenum::U32>,
    Item: Clone,
    F: KeyExtractor<Item, Key>,
{
    fn from_iter<I: IntoIterator<Item = Item>>(iter: I) -> Self {
        let mut tree = Self::new();
        for item in iter {
            let (new_tree, _) = tree.insert(item);
            tree = new_tree;
        }
        tree
    }
}

impl<Key, Item, F, Hash>  From<CompressedUtxoTree<Item>> for UtxoTree<Key, Item, F, Hash>
where
    Key: AsRef<[u8]> + Clone + std::hash::Hash + Eq,
    Hash: Digest<OutputSize = digest::typenum::U32>,
    Item: Clone,
    F: KeyExtractor<Item, Key>,
{
    fn from(compressed: CompressedUtxoTree<Item>) -> Self {
        Self {
            merkle: DynamicMerkleTree::from_compressed_tree::<Item, F>(&compressed),
            items: compressed.items.iter().map(|(pos, item)| {
                let key = F::extract(item);
                (key, (item.clone(), *pos))
            }).collect(),
            item_to_key: PhantomData,
        }
    }
}

#[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
pub struct CompressedUtxoTree<Item> {
    items: BTreeMap<usize, Item>,
}

#[cfg(feature = "serde")]
mod serde {
    use digest::Digest;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use crate::KeyExtractor;

    impl<Key, Item, F, Hash> Serialize for super::UtxoTree<Key, Item, F, Hash>
    where
        Key: Serialize + Clone + AsRef<[u8]> + std::hash::Hash + Eq,
        Item: Serialize + Clone,
        Hash: Digest<OutputSize = digest::typenum::U32>,
        F: KeyExtractor<Item, Key>,
    {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            self.compressed().serialize(serializer)
        }
    }

    impl<'de, Key, Item, F, Hash> Deserialize<'de> for super::UtxoTree<Key, Item, F, Hash>
    where
        Key: AsRef<[u8]> + Clone + std::hash::Hash + Eq + Deserialize<'de>,
        Item: Deserialize<'de> + Clone,
        Hash: Digest<OutputSize = digest::typenum::U32>,
        F: KeyExtractor<Item, Key>,
    {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let compressed = super::CompressedUtxoTree::<Item>::deserialize(deserializer)?;
            Ok(compressed.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::{Arbitrary, Gen};
    use quickcheck_macros::quickcheck;
    type TestHash = blake2::Blake2b<digest::typenum::U32>;

    #[derive(Clone, Debug)]
    struct IdentityKeyExtractor;
    impl<T: Clone> KeyExtractor<T, T> for IdentityKeyExtractor {
        fn extract(item: &T) -> T {
            item.clone()
        }
    }

    #[test]
    fn test_empty_tree() {
        let tree: UtxoTree<String, String, IdentityKeyExtractor, TestHash> = UtxoTree::new();
        assert_eq!(tree.size(), 0);
    }

    #[test]
    fn test_single_insert() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();
        let item = b"test_item".to_vec();
        let (tree_with_item, _pos) = tree.insert(item);

        assert_eq!(tree_with_item.size(), 1);
        assert_eq!(tree.size(), 0);
        assert_ne!(tree_with_item.root(), tree.root());
    }

    #[test]
    fn test_multiple_inserts() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        let items = [b"item1".to_vec(), b"item2".to_vec(), b"item3".to_vec()];
        let mut current_tree = tree;

        for (i, item) in items.iter().enumerate() {
            let (new_tree, pos) = current_tree.insert(item.clone());
            current_tree = new_tree;
            assert_eq!(current_tree.size(), i + 1);
            assert_eq!(pos, i);
        }

        assert_eq!(current_tree.size(), 3);
    }

    #[test]
    fn test_remove_existing_item() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        let item = b"test_item".to_vec();
        let key = &item;
        let (tree_with_item, _) = tree.insert(item.clone());

        let result = tree_with_item.remove(key);
        assert!(result.is_ok());

        let (tree_after_removal, removed_item) = result.unwrap();
        assert_eq!(tree_after_removal.size(), 0);
        assert_eq!(removed_item, item);
    }

    #[test]
    fn test_remove_non_existing_item() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        let item = b"test_item".to_vec();
        let (tree_with_item, _) = tree.insert(item);

        let non_existing_key = b"non_existing".to_vec();
        let result = tree_with_item.remove(&non_existing_key);
        assert!(matches!(result, Err(Error::NotFound)));
    }

    #[test]
    fn test_remove_from_empty_tree() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        let key = b"any_key".to_vec();
        let result = tree.remove(&key);
        assert!(matches!(result, Err(Error::NotFound)));
    }

    #[test]
    fn test_structural_sharing() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        let item1 = b"item1".to_vec();
        let item2 = b"item2".to_vec();

        let (tree1, _) = tree.insert(item1);
        let (tree2, _) = tree1.insert(item2);

        assert_eq!(tree.size(), 0);
        assert_eq!(tree1.size(), 1);
        assert_eq!(tree2.size(), 2);
    }

    #[test]
    fn test_root_changes_with_operations() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        let empty_root = tree.root();

        let item = b"test_item".to_vec();
        let (tree_with_item, _) = tree.insert(item.clone());
        let root_with_item = tree_with_item.root();

        assert_ne!(empty_root, root_with_item);

        let (tree_after_removal, _) = tree_with_item.remove(&item).unwrap();
        let root_after_removal = tree_after_removal.root();

        assert_eq!(empty_root, root_after_removal);
    }

    #[test]
    fn test_deterministic_root() {
        let tree1: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();
        let tree2: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        let items = vec![b"item1".to_vec(), b"item2".to_vec(), b"item3".to_vec()];

        let mut current_tree1 = tree1;
        let mut current_tree2 = tree2;

        for item in items {
            let (new_tree1, _) = current_tree1.insert(item.clone());
            let (new_tree2, _) = current_tree2.insert(item);
            current_tree1 = new_tree1;
            current_tree2 = new_tree2;
        }

        assert_eq!(current_tree1.root(), current_tree2.root());
    }

    #[test]
    fn test_mixed_operations() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        let mut current_tree = tree;
        let items = vec![
            b"item1".to_vec(),
            b"item2".to_vec(),
            b"item3".to_vec(),
            b"item4".to_vec(),
        ];

        for item in &items {
            let (new_tree, _) = current_tree.insert(item.clone());
            current_tree = new_tree;
        }
        assert_eq!(current_tree.size(), 4);

        let (tree_after_removal, _) = current_tree.remove(&items[1]).unwrap();
        assert_eq!(tree_after_removal.size(), 3);

        let (tree_after_removal2, _) = tree_after_removal.remove(&items[3]).unwrap();
        assert_eq!(tree_after_removal2.size(), 2);

        let new_item = b"new_item".to_vec();
        let (final_tree, _) = tree_after_removal2.insert(new_item);
        assert_eq!(final_tree.size(), 3);
    }

    #[test]
    fn test_empty_tree_root_consistency() {
        let tree1: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();
        let tree2: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        assert_eq!(tree1.root(), tree2.root());
    }

    #[test]
    fn test_position_tracking() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        let items = vec![b"item1".to_vec(), b"item2".to_vec(), b"item3".to_vec()];
        let mut current_tree = tree;
        let mut positions = Vec::new();

        for item in &items {
            let (new_tree, pos) = current_tree.insert(item.clone());
            current_tree = new_tree;
            positions.push(pos);
        }

        assert_eq!(positions, vec![0, 1, 2]);
    }

    #[test]
    fn test_large_tree_operations() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> = UtxoTree::new();

        let mut current_tree = tree;
        let num_items = 100;

        for i in 0..num_items {
            let item = format!("item_{i}").into_bytes();
            let (new_tree, pos) = current_tree.insert(item);
            current_tree = new_tree;
            assert_eq!(pos, i);
        }

        assert_eq!(current_tree.size(), num_items);

        for i in (0..num_items).step_by(2) {
            let key = format!("item_{i}").into_bytes();
            let result = current_tree.remove(&key);
            assert!(result.is_ok());
            let (new_tree, _) = result.unwrap();
            current_tree = new_tree;
        }

        assert_eq!(current_tree.size(), num_items / 2);
    }

    

    impl Arbitrary for UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash> {
        fn arbitrary(g: &mut Gen) -> Self {
            let num_items = usize::arbitrary(g) % 2 + 1; // 1-1000 items
            let mut tree: Self = Self::new();
            let mut items = (0..num_items)
                .map(|i| format!("item_{i}").into_bytes())
                .collect::<Vec<_>>();

            for item in &items {
               tree = tree.insert(item.clone()).0;
            }

            // Remove some items randomly
            let num_removals = usize::arbitrary(g) % num_items;
            for _ in 0..num_removals {
                let item = items.remove(usize::arbitrary(g) % items.len());
                tree = tree.remove(&item).unwrap().0;
            }

            tree
        }
    }

    #[quickcheck]
    fn test_compress_recover_roundtrip(test_tree: UtxoTree<Vec<u8>, Vec<u8>, IdentityKeyExtractor, TestHash>) -> bool {
        let original_tree = test_tree;

        // Compress the tree
        let compressed = original_tree.compressed();

        // Recover the tree from compressed format
        let recovered_tree: UtxoTree<_, _, _, _> = compressed.into();

        recovered_tree == original_tree 
    }
}
