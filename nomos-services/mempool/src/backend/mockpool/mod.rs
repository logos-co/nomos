use std::{
    collections::BTreeMap,
    convert::Infallible,
    fmt::Debug,
    hash::Hash,
    time::{SystemTime, UNIX_EPOCH},
};

use ::serde::{Deserialize, Serialize};
use linked_hash_map::LinkedHashMap;
use overwatch::services::state::ServiceState;

use super::Status;
use crate::{
    backend::{MemPool, MempoolError, RecoverableMempool},
    tx::settings::TxMempoolSettings,
};

mod serde;

/// A mock mempool implementation that stores all transactions in memory in the
/// order received.
#[derive(Serialize, Deserialize)]
pub struct MockPool<BlockId, Item, Key> {
    #[serde(
        serialize_with = "serde::serialize_pending_items",
        deserialize_with = "serde::deserialize_pending_items",
        bound(
            serialize = "Key: Eq + Hash + Serialize, Item: Serialize",
            deserialize = "Key: Deserialize<'de> + Eq + Hash, Item: Deserialize<'de>"
        )
    )]
    pending_items: LinkedHashMap<Key, Item>,
    #[serde(bound(deserialize = "BlockId: Ord + Deserialize<'de>"))]
    in_block_items: BTreeMap<BlockId, Vec<Item>>,
    #[serde(bound(deserialize = "Key: Ord + Deserialize<'de>"))]
    in_block_items_by_id: BTreeMap<Key, BlockId>,
    last_item_timestamp: u64,
}

impl<BlockId, Item, Key> MockPool<BlockId, Item, Key> {
    #[must_use]
    pub const fn pending_items(&self) -> &LinkedHashMap<Key, Item> {
        &self.pending_items
    }

    #[must_use]
    pub const fn in_block_items(&self) -> &BTreeMap<BlockId, Vec<Item>> {
        &self.in_block_items
    }

    #[must_use]
    pub const fn last_item_timestamp(&self) -> u64 {
        self.last_item_timestamp
    }
}

impl<BlockId, Item, Key> Debug for MockPool<BlockId, Item, Key>
where
    BlockId: Debug,
    Item: Debug,
    Key: Debug + Hash + Eq,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockPool")
            .field("pending_items", &self.pending_items)
            .field("in_block_items", &self.in_block_items)
            .field("in_block_items_by_id", &self.in_block_items_by_id)
            .field("last_item_timestamp", &self.last_item_timestamp)
            .finish()
    }
}

impl<BlockId, Item, Key> Default for MockPool<BlockId, Item, Key>
where
    Key: Hash + Eq,
{
    fn default() -> Self {
        Self {
            pending_items: LinkedHashMap::new(),
            in_block_items: BTreeMap::new(),
            in_block_items_by_id: BTreeMap::new(),
            last_item_timestamp: 0,
        }
    }
}

impl<BlockId, Item, Key> Clone for MockPool<BlockId, Item, Key>
where
    Key: Hash + Eq + Clone,
    Item: Clone,
    BlockId: Clone,
{
    fn clone(&self) -> Self {
        Self {
            pending_items: self.pending_items.clone(),
            in_block_items: self.in_block_items.clone(),
            in_block_items_by_id: self.in_block_items_by_id.clone(),
            last_item_timestamp: self.last_item_timestamp,
        }
    }
}

impl<BlockId, Item, Key> MockPool<BlockId, Item, Key>
where
    Key: Hash + Eq + Clone,
{
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<BlockId, Item, Key> MemPool for MockPool<BlockId, Item, Key>
where
    Key: Hash + Eq + Ord + Clone + Send,
    Item: Clone + Send + 'static,
    BlockId: Ord + Copy,
{
    type Settings = ();
    type Item = Item;
    type Key = Key;
    type BlockId = BlockId;

    fn new(_settings: Self::Settings) -> Self {
        Self::new()
    }

    fn add_item<I: Into<Self::Item>>(
        &mut self,
        key: Self::Key,
        item: I,
    ) -> Result<(), MempoolError> {
        if self.pending_items.contains_key(&key) || self.in_block_items_by_id.contains_key(&key) {
            return Err(MempoolError::ExistingItem);
        }
        self.pending_items.insert(key, item.into());
        self.last_item_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .try_into()
            .expect("Failed to convert timestamp to u64.");

        Ok(())
    }

    fn view(&self, _ancestor_hint: BlockId) -> Box<dyn Iterator<Item = Self::Item> + Send> {
        let pending_items: Vec<Item> = self.pending_items.values().cloned().collect();
        Box::new(pending_items.into_iter())
    }

    fn mark_in_block(&mut self, keys: &[Self::Key], block: BlockId) {
        let mut items_in_block = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(item) = self.pending_items.remove(key) {
                items_in_block.push(item);
            }
        }
        let block_entry = self.in_block_items.entry(block).or_default();
        self.in_block_items_by_id
            .extend(keys.iter().cloned().map(|key| (key, block)));
        block_entry.append(&mut items_in_block);
    }

    #[cfg(test)]
    fn block_items(&self, block: BlockId) -> Option<Box<dyn Iterator<Item = Self::Item> + Send>> {
        self.in_block_items.get(&block).map(|items| {
            Box::new(items.clone().into_iter()) as Box<dyn Iterator<Item = Self::Item> + Send>
        })
    }

    fn prune(&mut self, keys: &[Self::Key]) {
        for key in keys {
            self.pending_items.remove(key);
        }
    }

    fn pending_item_count(&self) -> usize {
        self.pending_items.len()
    }

    fn last_item_timestamp(&self) -> u64 {
        self.last_item_timestamp
    }

    fn status(&self, items: &[Self::Key]) -> Vec<Status<BlockId>> {
        items
            .iter()
            .map(|key| {
                if self.pending_items.contains_key(key) {
                    Status::Pending
                } else if let Some(block) = self.in_block_items_by_id.get(key) {
                    Status::InBlock { block: *block }
                } else {
                    Status::Unknown
                }
            })
            .collect()
    }
}

impl<BlockId, Item, Key> RecoverableMempool for MockPool<BlockId, Item, Key>
where
    Key: Hash + Eq + Ord + Clone + Send,
    Item: Clone + Send + 'static,
    BlockId: Ord + Copy,
{
    type RecoveryState = Self;

    fn recover(_settings: Self::Settings, state: Self::RecoveryState) -> Self {
        state
    }

    fn save(&self) -> Self::RecoveryState {
        self.clone()
    }
}

impl<BlockId, Item, Key> ServiceState for MockPool<BlockId, Item, Key>
where
    Key: Hash + Eq + Ord + Clone + Send,
    Item: Clone + Send + 'static,
    BlockId: Ord + Copy,
{
    type Error = Infallible;
    type Settings = TxMempoolSettings<(), (), ()>;

    fn from_settings(_settings: &Self::Settings) -> Result<Self, Self::Error> {
        Ok(<Self as MemPool>::new(()))
    }
}
