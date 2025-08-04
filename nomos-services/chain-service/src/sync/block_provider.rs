use std::{
    collections::{HashSet, VecDeque},
    fmt::Debug,
    marker::PhantomData,
    num::NonZeroUsize,
    ops::RangeInclusive,
};

use bytes::Bytes;
use cryptarchia_engine::{Branch, Slot};
use futures::{future, stream, stream::BoxStream, StreamExt as _, TryStreamExt as _};
use nomos_core::{block::Block, header::HeaderId, wire};
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend, StorageMsg};
use overwatch::DynError;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::{mpsc::Sender, oneshot};
use tracing::{error, info};

use crate::relays::StorageRelay;

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

#[derive(Debug, Clone)]
struct BlockInfo {
    id: HeaderId,
    slot: Slot,
    location: BlockLocation,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum BlockLocation {
    Engine,
    Storage,
}

pub struct BlockProvider<Storage, Tx, BlobCertificate>
where
    Storage: StorageBackend,
    Tx: Clone + Eq,
    BlobCertificate: Clone + Eq,
{
    storage_relay: StorageRelay<Storage>,
    _phantom: PhantomData<(Tx, BlobCertificate)>,
}

impl<Storage, Tx, BlobCertificate> BlockProvider<Storage, Tx, BlobCertificate>
where
    Storage: StorageBackend + 'static,
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
        cryptarchia: &cryptarchia_engine::Cryptarchia<HeaderId>,
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
        cryptarchia: &cryptarchia_engine::Cryptarchia<HeaderId>,
    ) -> Result<BoxStream<'static, Result<Bytes, DynError>>, DynError> {
        let path = self
            .find_path(cryptarchia, target_block, known_blocks)
            .await?;

        let stream = self.stream_blocks_from_path(path);
        Ok(stream)
    }

    /// Creates a stream from a computed path of block IDs
    fn stream_blocks_from_path(
        &self,
        path: Vec<HeaderId>,
    ) -> BoxStream<'static, Result<Bytes, DynError>> {
        let storage = self.storage_relay.clone();

        let stream = stream::iter(path)
            .then(move |id| {
                let storage = storage.clone();

                async move {
                    let block = Self::load_block(id, &storage)
                        .await
                        .map_err(DynError::from)?
                        .ok_or_else(|| DynError::from(GetBlocksError::BlockNotFound(id)))?;

                    Ok::<_, DynError>(Bytes::from(
                        wire::serialize(&block).expect("Block must be serialized"),
                    ))
                }
            })
            .map_err(DynError::from)
            .take_while(|result| future::ready(result.is_ok()));

        Box::pin(stream)
    }

    /// Finds the path from one of the known blocks to the target block
    async fn find_path(
        &self,
        cryptarchia: &cryptarchia_engine::Cryptarchia<HeaderId>,
        target_block: HeaderId,
        known_blocks: &HashSet<HeaderId>,
    ) -> Result<Vec<HeaderId>, GetBlocksError> {
        let target_info = self.find_target_block(cryptarchia, target_block).await?;

        let start_info = self
            .find_optimal_start_block(cryptarchia, known_blocks, &target_info)
            .await?;

        self.compute_path_between_endpoints(cryptarchia, start_info, target_info)
            .await
    }

    /// Locates the target block and determines its slot and location
    async fn find_target_block(
        &self,
        cryptarchia: &cryptarchia_engine::Cryptarchia<HeaderId>,
        target_block: HeaderId,
    ) -> Result<BlockInfo, GetBlocksError> {
        if let Some(target_branch) = cryptarchia.branches().get(&target_block) {
            return Ok(BlockInfo {
                id: target_block,
                slot: target_branch.slot(),
                location: BlockLocation::Engine,
            });
        }

        if let Some(target_storage_block) = self.load_immutable_block(target_block).await? {
            return Ok(BlockInfo {
                id: target_block,
                slot: target_storage_block.header().slot(),
                location: BlockLocation::Storage,
            });
        }

        Err(GetBlocksError::BlockNotFound(target_block))
    }

    /// Finds the optimal starting block from the given [`known_blocks`]
    /// for building a block stream that leads to the target block
    /// indicated by [`target_info`].
    ///
    /// This function first tries to find the optimal starting block
    /// from the engine, if the target block and some of the
    /// [`known_blocks`] are present in the engine.
    /// If not, it returns the most recent block among the [`known_blocks`]
    /// stored as immutable blocks in the storage.
    ///
    /// In any error case, [`GetBlocksError`] is returned.
    /// If the [`target_block`] is not found in the engine and the storage,
    /// [`GetBlocksError::BlockNotFound`] is returned.
    /// If none of the [`known_blocks`] is found in the engine and the storage,
    /// [`GetBlocksError::StartBlockNotFound`] is returned.
    async fn find_optimal_start_block(
        &self,
        cryptarchia: &cryptarchia_engine::Cryptarchia<HeaderId>,
        known_blocks: &HashSet<HeaderId>,
        target_info: &BlockInfo,
    ) -> Result<BlockInfo, GetBlocksError> {
        if target_info.location == BlockLocation::Engine {
            if let Some(start_info) =
                Self::find_optimal_start_block_from_engine(cryptarchia, known_blocks, target_info)?
            {
                return Ok(start_info);
            }
        }

        self.find_optimal_start_block_from_storage(known_blocks)
            .await?
            .ok_or(GetBlocksError::StartBlockNotFound)
    }

    fn find_optimal_start_block_from_engine(
        cryptarchia: &cryptarchia_engine::Cryptarchia<HeaderId>,
        known_blocks: &HashSet<HeaderId>,
        target_info: &BlockInfo,
    ) -> Result<Option<BlockInfo>, GetBlocksError> {
        let target_block = cryptarchia
            .branches()
            .get(&target_info.id)
            .ok_or(GetBlocksError::BlockNotFound(target_info.id))?;

        if let Some(lca) = Self::max_lca(cryptarchia, target_block, known_blocks) {
            return Ok(Some(BlockInfo {
                id: lca.id(),
                slot: lca.slot(),
                location: BlockLocation::Engine,
            }));
        }

        Ok(None)
    }

    async fn find_optimal_start_block_from_storage(
        &self,
        known_blocks: &HashSet<HeaderId>,
    ) -> Result<Option<BlockInfo>, GetBlocksError> {
        if let Some(immutable_block) = self
            .find_max_slot_immutable_block(known_blocks.iter().copied())
            .await?
        {
            return Ok(Some(BlockInfo {
                id: immutable_block.header().id(),
                slot: immutable_block.header().slot(),
                location: BlockLocation::Storage,
            }));
        }

        Ok(None)
    }

    async fn compute_path_between_endpoints(
        &self,
        cryptarchia: &cryptarchia_engine::Cryptarchia<HeaderId>,
        start_info: BlockInfo,
        target_info: BlockInfo,
    ) -> Result<Vec<HeaderId>, GetBlocksError> {
        let limit = MAX_NUMBER_OF_BLOCKS
            .try_into()
            .expect("MAX_NUMBER_OF_BLOCKS should be > 0");

        match start_info.location {
            BlockLocation::Engine => {
                Self::compute_path_from_engine(cryptarchia, start_info.id, target_info.id, limit)
                    .map(Into::into)
            }

            BlockLocation::Storage => {
                self.compute_path_from_storage_and_engine(
                    cryptarchia,
                    start_info.slot,
                    target_info.id,
                    target_info.slot,
                    limit,
                )
                .await
            }
        }
    }

    fn compute_path_from_engine(
        cryptarchia: &cryptarchia_engine::Cryptarchia<HeaderId>,
        start_block: HeaderId,
        target_block: HeaderId,
        limit: NonZeroUsize,
    ) -> Result<VecDeque<HeaderId>, GetBlocksError> {
        let mut path = VecDeque::new();
        let branches = cryptarchia.branches();

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

    /// Builds a list of block IDs using storage scan + engine path when needed
    async fn compute_path_from_storage_and_engine(
        &self,
        cryptarchia: &cryptarchia_engine::Cryptarchia<HeaderId>,
        start_block_slot: Slot,
        target_block: HeaderId,
        target_block_slot: Slot,
        limit: NonZeroUsize,
    ) -> Result<Vec<HeaderId>, GetBlocksError> {
        let mut storage_path = self
            .scan_immutable_block_ids(start_block_slot..=target_block_slot, limit)
            .await?;

        if storage_path.len() >= limit.get() {
            return Ok(storage_path);
        }

        let remaining_limit = NonZeroUsize::new(limit.get() - storage_path.len())
            .expect("Remaining limit should be > 0");

        let engine_path = Self::compute_path_from_engine(
            cryptarchia,
            cryptarchia.lib(),
            target_block,
            remaining_limit,
        )?;

        storage_path.extend(engine_path.iter());

        Ok(storage_path)
    }

    fn max_lca(
        cryptarchia: &cryptarchia_engine::Cryptarchia<HeaderId>,
        target_branch: &Branch<HeaderId>,
        known_blocks: &HashSet<HeaderId>,
    ) -> Option<Branch<HeaderId>> {
        let branches = cryptarchia.branches();
        known_blocks
            .iter()
            .filter_map(|known| {
                branches
                    .get(known)
                    .map(|known_branch| branches.lca(known_branch, target_branch))
            })
            .max_by_key(Branch::length)
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
        let Some(block) = Self::load_block(id, &self.storage_relay).await? else {
            return Ok(None);
        };

        // Check if the block is stored as immutable in the storage
        let (tx, rx) = oneshot::channel();
        self.storage_relay
            .send(StorageMsg::get_immutable_block_id_request(
                block.header().slot(),
                tx,
            ))
            .await
            .map_err(|(e, _)| GetBlocksError::SendError(e.to_string()))?;

        match rx.await.map_err(|_| GetBlocksError::ChannelDropped)? {
            Some(_) => Ok(Some(block)),
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

        let response = rx.await.map_err(|_| GetBlocksError::ChannelDropped)?;

        match response {
            None => Ok(None),
            Some(block) => Ok(Some(
                block
                    .try_into()
                    .map_err(|_| GetBlocksError::ConversionError)?,
            )),
        }
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
