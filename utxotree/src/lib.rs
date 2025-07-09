mod merkle;

use digest::Digest;
use merkle::DynamicMerkleTree;
use rpds::HashTrieMap;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct UtxoTree<Key, Item, F, Hash>
where
    Key: std::hash::Hash + Eq,
{
    merkle: DynamicMerkleTree<Key, Hash>,
    // key -> (item, position in merkle tree)
    items: HashTrieMap<Key, (Item, usize)>,
    item_to_key: F,
}

impl<Key, Item, F, Hash> UtxoTree<Key, Item, F, Hash>
where
    Key: AsRef<[u8]> + Clone + std::hash::Hash + Eq,
    Hash: Digest<OutputSize = digest::typenum::U32>,
{
    pub fn new(item_to_key: F) -> Self {
        Self {
            merkle: DynamicMerkleTree::new(),
            items: HashTrieMap::new(),
            item_to_key,
        }
    }

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
    F: Fn(&Item) -> Key + Clone,
    Item: Clone,
{
    pub fn insert(&self, item: Item) -> (Self, usize) {
        let key = (self.item_to_key)(&item);
        let (merkle, pos) = self.merkle.insert(key.clone());
        let items = self.items.insert(key, (item, pos));
        (
            Self {
                merkle,
                items,
                item_to_key: self.item_to_key.clone(),
            },
            pos,
        )
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
                item_to_key: self.item_to_key.clone(),
            },
            item.clone(),
        ))
    }

    pub fn root(&self) -> [u8; 32] {
        self.merkle.root()
    }

    pub fn witness(&self, _index: usize) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestHash = blake2::Blake2b<digest::typenum::U32>;

    #[test]
    fn test_empty_tree() {
        let tree: UtxoTree<String, (), _, TestHash> = UtxoTree::new(|_item: &()| String::new());
        assert_eq!(tree.size(), 0);
    }

    #[test]
    fn test_single_insert() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());
        let item = b"test_item".to_vec();
        let (tree_with_item, _pos) = tree.insert(item);

        assert_eq!(tree_with_item.size(), 1);
        assert_eq!(tree.size(), 0);
        assert_ne!(tree_with_item.root(), tree.root());
    }

    #[test]
    fn test_multiple_inserts() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

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
        let tree: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

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
        let tree: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

        let item = b"test_item".to_vec();
        let (tree_with_item, _) = tree.insert(item);

        let non_existing_key = b"non_existing".to_vec();
        let result = tree_with_item.remove(&non_existing_key);
        assert!(matches!(result, Err(Error::NotFound)));
    }

    #[test]
    fn test_remove_from_empty_tree() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

        let key = b"any_key".to_vec();
        let result = tree.remove(&key);
        assert!(matches!(result, Err(Error::NotFound)));
    }

    #[test]
    fn test_structural_sharing() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

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
        let tree: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

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
        let tree1: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());
        let tree2: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

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
    fn test_custom_key_extractor() {
        #[derive(Clone)]
        struct TestItem {
            id: u32,
            data: String,
        }

        let tree: UtxoTree<Vec<u8>, TestItem, _, TestHash> =
            UtxoTree::new(|item: &TestItem| item.id.to_be_bytes().to_vec());

        let item = TestItem {
            id: 42,
            data: "test data".to_owned(),
        };

        let (tree_with_item, _) = tree.insert(item);
        assert_eq!(tree_with_item.size(), 1);

        let result = tree_with_item.remove(&42u32.to_be_bytes().to_vec());
        assert!(result.is_ok());

        let (tree_after_removal, removed_item) = result.unwrap();
        assert_eq!(tree_after_removal.size(), 0);
        assert_eq!(removed_item.id, 42);
        assert_eq!(removed_item.data, "test data");
    }

    #[test]
    fn test_mixed_operations() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

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
        let tree1: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());
        let tree2: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

        assert_eq!(tree1.root(), tree2.root());
    }

    #[test]
    fn test_position_tracking() {
        let tree: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

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
        let tree: UtxoTree<Vec<u8>, Vec<u8>, _, TestHash> =
            UtxoTree::new(|item: &Vec<u8>| item.clone());

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
}
