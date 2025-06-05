use crate::relays::StorageRelay;

pub struct BlockProvider<Storage> {
    storage_relay: StorageRelay<Storage>,
}

impl<Storage> BlockProvider<Storage>
where
    Storage: Send + Sync + 'static,
{
    pub fn new(storage_relay: StorageRelay<Storage>) -> Self {
        Self { storage_relay }
    }

    /// Input
    /// Peer node tries to sync up to target block(our local tip).
    ///
    /// (The responding peer finds the latest common ancestor (i.e. LCA) between the target_block and each of the known blocks.)
    ///
    /// - `target_block: HeaderId`
    /// - `known_blocks: Vec<HeaderId>`
    /// - `local_tip: HeaderId`
    /// - `latest_immutable_block: HeaderId`
    ///
    /// /// Output
    /// Batch of blocks
    pub async fn get_blocks(&self) -> Option<Storage> {}
}
