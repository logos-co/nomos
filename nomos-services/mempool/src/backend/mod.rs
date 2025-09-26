#[cfg(feature = "mock")]
pub mod mockpool;

use serde::{Deserialize, Serialize};

#[derive(thiserror::Error, Debug)]
pub enum MempoolError {
    #[error("Item already in mempool")]
    ExistingItem,
    #[error(transparent)]
    DynamicPoolError(#[from] overwatch::DynError),
}

pub trait Mempool {
    type Settings;
    type Item;
    type Key;
    type BlockId;

    /// Construct a new empty pool
    fn new(settings: Self::Settings) -> Self;

    /// Add a new item to the mempool, for example because we received it from
    /// the network
    fn add_item<I: Into<Self::Item>>(
        &mut self,
        key: Self::Key,
        item: I,
    ) -> Result<(), MempoolError>;

    /// Return a view over items contained in the mempool.
    /// Implementations should provide *at least* all the items which have not
    /// been marked as in a block.
    /// The hint on the ancestor *can* be used by the implementation to display
    /// additional items that were not included up to that point if
    /// available.
    fn view(&self, ancestor_hint: Self::BlockId) -> Box<dyn Iterator<Item = Self::Item> + Send>;

    /// Record that a set of items were included in a block
    fn mark_in_block(&mut self, items: &[Self::Key], block: Self::BlockId);

    /// Returns all of the transactions for the block
    #[cfg(test)]
    fn block_items(
        &self,
        block: Self::BlockId,
    ) -> Option<Box<dyn Iterator<Item = Self::Item> + Send>>;

    /// Signal that a set of transactions can't be possibly requested anymore
    /// and can be discarded.
    fn prune(&mut self, items: &[Self::Key]);

    fn pending_item_count(&self) -> usize;
    fn last_item_timestamp(&self) -> u64;

    // Return the status of a set of items.
    // This is a best effort attempt, and implementations are free to return
    // `Unknown` for all of them.
    fn status(&self, items: &[Self::Key]) -> Vec<Status<Self::BlockId>>;
}

/// A mempool with recovery capabilities.
pub trait RecoverableMempool: Mempool {
    /// The state that is serialized and deserialized and from which the pool
    /// state is recreated.
    type RecoveryState;

    /// Restore the mempool from the provided settings and state.
    fn recover(settings: Self::Settings, state: Self::RecoveryState) -> Self;
    /// Save some state for later recovery.
    fn save(&self) -> Self::RecoveryState;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum Status<BlockId> {
    /// Unknown status
    Unknown,
    /// Pending status
    Pending,
    /// Rejected status
    Rejected,
    /// Accepted status
    ///
    /// The block id of the block that contains the item
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "e.g. 0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
        )
    )]
    InBlock { block: BlockId },
}
