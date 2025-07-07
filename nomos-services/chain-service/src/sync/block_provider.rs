use std::{
    collections::{HashSet, VecDeque},
    fmt::Debug,
    hash::Hash,
    marker::PhantomData,
    num::NonZeroUsize,
    ops::RangeInclusive,
};

use bytes::Bytes;
use cryptarchia_engine::{Branch, Branches, CryptarchiaState, Slot};
use futures::{future, stream, stream::BoxStream, StreamExt as _, TryStreamExt as _};
use nomos_core::{block::Block, header::HeaderId, wire};
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend, StorageMsg};
use overwatch::DynError;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::{mpsc::Sender, oneshot};
use tracing::{error, info};

use crate::{relays::StorageRelay, Cryptarchia};

const MAX_NUMBER_OF_BLOCKS: usize = 1000;

#[derive(Debug, Error)]
pub enum GetBlocksError {
    #[error("Storage channel dropped")]
    ChannelDropped,
    #[error("Block not found in storage for header {0:?}")]
    BlockNotFound(HeaderId),
    #[error("Failed to find start block")]
    StartBlockNotFound,
    #[error("Invalid state: {0}")]
    InvalidState(String),
    #[error("Failed to send to channel: {0}")]
    SendError(String),
    #[error("Failed to convert block")]
    ConversionError,
}

pub struct BlockProvider<Storage, State, Tx, BlobCertificate>
where
    Storage: StorageBackend,
{
    storage_relay: StorageRelay<Storage>,
    _phantom: PhantomData<(State, Tx, BlobCertificate)>,
}

impl<Storage, State, Tx, BlobCertificate> BlockProvider<Storage, State, Tx, BlobCertificate>
where
    Storage: StorageBackend + 'static,
    State: CryptarchiaState + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block: TryInto<Block<Tx, BlobCertificate>>,
    Tx: Serialize + Clone + Eq + Send + Sync + 'static,
    BlobCertificate: Serialize + Clone + Eq + Send + Sync + 'static,
{
    pub const fn new(storage_relay: StorageRelay<Storage>) -> Self {
        Self {
            storage_relay,
            _phantom: PhantomData,
        }
    }

    /// Creates a block stream that leads from one of the [`known_blocks`]
    /// to the [`target_block`], and sends it to the [`reply_sender`].
    pub async fn send_blocks(
        &self,
        cryptarchia: &Cryptarchia<State>,
        target_block: HeaderId,
        known_blocks: &HashSet<HeaderId>,
        reply_sender: Sender<BoxStream<'static, Result<Bytes, DynError>>>,
    ) {
        info!(
            "Providing blocks:
            target_block={target_block:?},
            known_blocks={known_blocks:?},"
        );

        match self
            .block_stream(target_block, known_blocks, cryptarchia)
            .await
        {
            Ok(stream) => {
                if let Err(e) = reply_sender.send(stream).await {
                    error!("Failed to send blocks stream: {e}");
                }
            }
            Err(e) => {
                Self::send_error(
                    format!("Failed to create a block stream: {e:?}"),
                    reply_sender,
                )
                .await;
            }
        }
    }

    /// Creates a block stream that leads from one of the [`known_blocks`]
    /// to the [`target_block`], in parent-to-child order.
    ///
    /// The stream includes at most [`MAX_NUMBER_OF_BLOCKS`] blocks.
    /// The stream may or may not reach the [`target_block`].
    ///
    /// In any error case, [`GetBlocksError`] is returned.
    /// If the [`target_block`] is not found in the engine and the storage,
    /// [`GetBlocksError::BlockNotFound`] is returned.
    /// If none of the [`known_blocks`] is found in the engine and the storage,
    /// [`GetBlocksError::StartBlockNotFound`] is returned.
    async fn block_stream(
        &self,
        target_block: HeaderId,
        known_blocks: &HashSet<HeaderId>,
        cryptarchia: &Cryptarchia<State>,
    ) -> Result<BoxStream<'static, Result<Bytes, DynError>>, GetBlocksError> {
        // Find the optimal starting block.
        let (start_block_id, start_block_slot, target_block_slot) = self
            .find_start_block(target_block, known_blocks, cryptarchia.consensus.branches())
            .await?
            .ok_or(GetBlocksError::StartBlockNotFound)?;

        // Compute a list of block IDs to be returned.
        let path = self
            .compute_path(
                start_block_id,
                start_block_slot,
                target_block,
                target_block_slot,
                MAX_NUMBER_OF_BLOCKS
                    .try_into()
                    .expect("MAX_NUMBER_OF_BLOCKS should be > 0"),
                cryptarchia,
            )
            .await?;

        // Here we can't return a stream from storage because blocks aren't ordered by
        // their IDs in storage.
        let storage = self.storage_relay.clone();
        let stream = stream::iter(path)
            .then(move |id| {
                let storage = storage.clone();
                async move {
                    let block = Self::load_block(id, &storage)
                        .await?
                        .ok_or(GetBlocksError::BlockNotFound(id))?;
                    Ok(Bytes::from(
                        wire::serialize(&block).expect("Block must be serialized"),
                    ))
                }
            })
            .map_err(|e: GetBlocksError| {
                error!("Error processing block: {e}");
                DynError::from(e)
            })
            .take_while(|result| future::ready(result.is_ok()));
        Ok(Box::pin(stream))
    }

    /// Finds the optimal starting block from the given [`known_blocks`]
    /// for building a block stream that leads to the [`target_block`].
    ///
    /// This function first tries to find the optimal starting block
    /// from the engine, if the [`target_block`] and some of the
    /// [`known_blocks`] are present in the engine.
    /// If not, it returns the most recent block among the [`known_blocks`]
    /// stored as immutable blocks in the storage.
    ///
    /// In any error case, [`GetBlocksError`] is returned.
    /// If the [`target_block`] is not found in the engine and the storage,
    /// [`GetBlocksError::BlockNotFound`] is returned.
    async fn find_start_block(
        &self,
        target_block: HeaderId,
        known_blocks: &HashSet<HeaderId>,
        branches: &Branches<HeaderId>,
    ) -> Result<Option<(HeaderId, Slot, Slot)>, GetBlocksError> {
        // If the target is in the engine,
        if let Some(target) = branches.get(&target_block) {
            // First, try to find the max LCA from engine.
            if let Some(max_lca) = max_lca(branches, target, known_blocks) {
                return Ok(Some((max_lca.id(), max_lca.slot(), target.slot())));
            }

            // If no LCA found, return the most recent immutable block
            // among [`known_blocks`].
            return Ok(self
                .find_max_slot_immutable_block(known_blocks.iter().copied())
                .await?
                .map(|block| (block.header().id(), block.header().slot(), target.slot())));
        }

        // Check whether the target block is stored as immutable in the storage.
        match self.load_immutable_block(target_block).await? {
            Some(target_block) => {
                // Return the most recent immutable block among [`known_blocks`].
                Ok(self
                    .find_max_slot_immutable_block(known_blocks.iter().copied())
                    .await?
                    .map(|block| {
                        (
                            block.header().id(),
                            block.header().slot(),
                            target_block.header().slot(),
                        )
                    }))
            }
            None => Err(GetBlocksError::BlockNotFound(target_block)),
        }
    }

    async fn find_max_slot_immutable_block(
        &self,
        ids: impl Iterator<Item = HeaderId>,
    ) -> Result<Option<Block<Tx, BlobCertificate>>, GetBlocksError> {
        Ok(self
            .load_immutable_blocks(ids)
            .await?
            .iter()
            .max_by_key(|block| block.header().slot())
            .cloned())
    }

    async fn load_immutable_blocks(
        &self,
        ids: impl Iterator<Item = HeaderId>,
    ) -> Result<Vec<Block<Tx, BlobCertificate>>, GetBlocksError> {
        let mut blocks = Vec::new();
        for id in ids {
            if let Some(block) = self.load_immutable_block(id).await? {
                blocks.push(block);
            }
        }
        Ok(blocks)
    }

    /// Loads an immutable block by its ID from the storage.
    /// If the block is not found, or if it is not stored as immutable,
    /// returns [`None`].
    async fn load_immutable_block(
        &self,
        id: HeaderId,
    ) -> Result<Option<Block<Tx, BlobCertificate>>, GetBlocksError> {
        // First, load the block from storage by the given ID.
        match Self::load_block(id, &self.storage_relay).await? {
            Some(block) => {
                // Check if the block is stored as immutable in the storage.
                let (tx, rx) = oneshot::channel();
                self.storage_relay
                    .send(StorageMsg::get_immutable_block_id_request(
                        block.header().slot(),
                        tx,
                    ))
                    .await
                    .map_err(|(e, _)| GetBlocksError::SendError(e.to_string()))?;
                Ok(rx
                    .await
                    .map_err(|_| GetBlocksError::ChannelDropped)?
                    .map(|_| block))
            }
            None => Ok(None),
        }
    }

    /// Loads a block from storage, regardless of whether it is stored as
    /// immutable or not.
    async fn load_block(
        id: HeaderId,
        storage: &StorageRelay<Storage>,
    ) -> Result<Option<Block<Tx, BlobCertificate>>, GetBlocksError> {
        let (tx, rx) = oneshot::channel();
        storage
            .send(StorageMsg::get_block_request(id, tx))
            .await
            .map_err(|(e, _)| GetBlocksError::SendError(e.to_string()))?;
        match rx.await.map_err(|_| GetBlocksError::ChannelDropped)? {
            Some(block) => Ok(Some(
                block
                    .try_into()
                    .map_err(|_| GetBlocksError::ConversionError)?,
            )),
            None => Ok(None),
        }
    }

    /// Builds a list of block IDs that lead from the `start` block
    /// to the `target` block, up to the given `limit`.
    ///
    /// This function accesses both the engine and the storage.
    async fn compute_path(
        &self,
        start_block: HeaderId,
        start_block_slot: Slot,
        target_block: HeaderId,
        target_block_slot: Slot,
        limit: NonZeroUsize,
        cryptarchia: &Cryptarchia<State>,
    ) -> Result<Vec<HeaderId>, GetBlocksError> {
        // If `start_block` is in the engine, compute the path from the engine.
        if cryptarchia.consensus.branches().get(&start_block).is_some() {
            compute_path_from_engine(
                cryptarchia.consensus.branches(),
                start_block,
                target_block,
                limit,
            )
            .map(Into::into)
        } else {
            // If not, use the storage + the engine.
            self.compute_path_from_storage_and_engine(
                start_block_slot,
                target_block,
                target_block_slot,
                limit,
                cryptarchia,
            )
            .await
        }
    }

    /// Builds a list of block IDs that lead from the block at
    /// `start_block_slot` to the `target` block, limited to `limit`.
    ///
    /// This function first scans immutable blocks from the storage,
    /// starting from the `start` block.
    /// If the scanned blocks are not enough to reach the `limit`,
    /// it computes the rest of the path from the engine.
    async fn compute_path_from_storage_and_engine(
        &self,
        start_block_slot: Slot,
        target_block: HeaderId,
        target_block_slot: Slot,
        limit: NonZeroUsize,
        cryptarchia: &Cryptarchia<State>,
    ) -> Result<Vec<HeaderId>, GetBlocksError> {
        // First, scan immutable blocks from the storage.
        let mut path = self
            .scan_immutable_block_ids(
                RangeInclusive::new(start_block_slot, target_block_slot),
                limit,
            )
            .await?;

        // If the `limit` is not reached, compute the rest of the path from the engine.
        if path.len() < limit.get() {
            path.extend(
                compute_path_from_engine(
                    cryptarchia.consensus.branches(),
                    // Start from the LIB which must be the oldest block in the engine.
                    cryptarchia.consensus.lib(),
                    target_block,
                    limit
                        .get()
                        .checked_sub(path.len())
                        .expect("limit must be > path.len()")
                        .try_into()
                        .expect("limit - path.len() must be > 0"),
                )?
                .iter(),
            );
        }
        Ok(path)
    }

    /// Scans immutable block IDs from the storage,
    /// starting from the `start_slot`, limited to `limit`.
    async fn scan_immutable_block_ids(
        &self,
        slot_range: RangeInclusive<Slot>,
        limit: NonZeroUsize,
    ) -> Result<Vec<HeaderId>, GetBlocksError> {
        let (tx, rx) = oneshot::channel();
        self.storage_relay
            .send(StorageMsg::scan_immutable_block_ids_request(
                slot_range, limit, tx,
            ))
            .await
            .map_err(|(e, _)| GetBlocksError::SendError(e.to_string()))?;
        rx.await.map_err(|_| GetBlocksError::ChannelDropped)
    }

    async fn send_error(msg: String, reply_sender: Sender<BoxStream<'_, Result<Bytes, DynError>>>) {
        error!(msg);

        let stream = stream::once(async move { Err(DynError::from(msg)) });
        if let Err(e) = reply_sender
            .send(Box::pin(stream))
            .await
            .map_err(|_| GetBlocksError::SendError("Failed to send error stream".to_owned()))
        {
            error!("Failed to send error stream: {e}");
        }
    }
}

fn max_lca<Id>(
    branches: &Branches<Id>,
    target_branch: &Branch<Id>,
    known_blocks: &HashSet<Id>,
) -> Option<Branch<Id>>
where
    Id: Hash + Eq + Copy + Debug,
{
    known_blocks
        .iter()
        .filter_map(|known| {
            branches
                .get(known)
                .map(|known_branch| branches.lca(known_branch, target_branch))
        })
        .max_by_key(Branch::length)
}

/// Computes a path from the `start_block` to the `target_block` in the
/// `branches`. If any block between `start_block` and `target_block` is not
/// found in the `branches`, [`GetBlocksError::InvalidState`] is returned.
fn compute_path_from_engine<Id>(
    branches: &Branches<Id>,
    start_block: Id,
    target_block: Id,
    limit: NonZeroUsize,
) -> Result<VecDeque<Id>, GetBlocksError>
where
    Id: Copy + Eq + Hash + Debug,
{
    let mut path = VecDeque::new();

    let mut current = target_block;
    loop {
        path.push_front(current);

        if path.len() > limit.get() {
            path.pop_back();
        }

        if current == start_block {
            return Ok(path);
        }

        match branches.get(&current).map(Branch::parent) {
            Some(parent) => {
                if parent == current {
                    return Err(GetBlocksError::InvalidState(format!(
                        "Genesis block reached before reaching start_block: {start_block:?}"
                    )));
                }
                current = parent;
            }
            None => {
                return Err(GetBlocksError::InvalidState(format!(
                    "Couldn't reach start_block: {start_block:?}"
                )));
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::num::NonZero;

    use cryptarchia_engine::{Boostrapping, Config};

    use super::*;

    #[test]
    fn test_compute_path_from_engine() {
        let mut cryptarchia = new_cryptarchia();

        cryptarchia = cryptarchia
            .receive_block([1; 32], [0; 32], Slot::from(1))
            .expect("Failed to add block")
            .0;

        cryptarchia = cryptarchia
            .receive_block([2; 32], [1; 32], Slot::from(2))
            .expect("Failed to add block")
            .0;

        let branches = cryptarchia.branches();

        let start_block = [1; 32];
        let target_block = [2; 32];

        let path = compute_path_from_engine(
            branches,
            start_block,
            target_block,
            MAX_NUMBER_OF_BLOCKS.try_into().unwrap(),
        )
        .unwrap();

        assert_eq!(path.len(), 2);
        assert_eq!(path.front().unwrap(), &start_block);
        assert_eq!(path.back().unwrap(), &target_block);
    }

    #[test]
    fn test_compute_path_from_engine_limit_bounds() {
        let mut cryptarchia = new_cryptarchia();

        cryptarchia = cryptarchia
            .receive_block([1; 32], [0; 32], Slot::from(1))
            .expect("Failed to add block")
            .0;

        cryptarchia = cryptarchia
            .receive_block([2; 32], [1; 32], Slot::from(2))
            .expect("Failed to add block")
            .0;

        cryptarchia = cryptarchia
            .receive_block([3; 32], [2; 32], Slot::from(3))
            .expect("Failed to add block")
            .0;

        let branches = cryptarchia.branches();

        let start_block = [1; 32];
        let target_block = [3; 32];
        let last_block_in_computed_path = [2; 32];

        let limit = 2;

        let path = compute_path_from_engine(
            branches,
            start_block,
            target_block,
            limit.try_into().unwrap(),
        )
        .unwrap();

        assert_eq!(path.len(), limit);
        assert_eq!(path.front().unwrap(), &start_block);
        assert_eq!(path.back().unwrap(), &last_block_in_computed_path);
    }

    #[test]
    fn test_compute_path_from_engine_unreachable_start() {
        let cryptarchia = new_cryptarchia();

        let start_block = [1; 32];
        let target_block = [2; 32];

        let branches = cryptarchia.branches();

        let path = compute_path_from_engine(
            branches,
            start_block,
            target_block,
            MAX_NUMBER_OF_BLOCKS.try_into().unwrap(),
        );

        assert!(matches!(
                    path,
                    Err(GetBlocksError::InvalidState(ref msg)) if
        msg.contains("Couldn't reach start_block")
                ));
    }

    #[test]
    fn test_compute_path_from_engine_hits_genesis() {
        let mut cryptarchia = new_cryptarchia();

        cryptarchia = cryptarchia
            .receive_block([3; 32], [0; 32], Slot::from(1))
            .expect("Failed to add block")
            .0;

        let branches = cryptarchia.branches();

        let start_block_not_existing = [2; 32];
        let target_block = [3; 32];

        let path = compute_path_from_engine(
            branches,
            start_block_not_existing,
            target_block,
            MAX_NUMBER_OF_BLOCKS.try_into().unwrap(),
        );

        assert!(matches!(
            path,
            Err(GetBlocksError::InvalidState(ref msg)) if
msg.contains("Genesis block reached")         ));
    }

    fn new_cryptarchia() -> cryptarchia_engine::Cryptarchia<[u8; 32], Boostrapping> {
        <cryptarchia_engine::Cryptarchia<_, Boostrapping>>::from_lib(
            [0; 32],
            Config {
                security_param: NonZero::new(1).unwrap(),
                active_slot_coeff: 1.0,
            },
        )
    }
}
