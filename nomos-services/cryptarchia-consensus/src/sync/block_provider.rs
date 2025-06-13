use std::{collections::VecDeque, hash::Hash, marker::PhantomData};

use cryptarchia_engine::CryptarchiaState;
use futures::{stream, stream::BoxStream, StreamExt, TryFutureExt};
use nomos_core::{block::Block, header::HeaderId, wire};
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend, StorageMsg};
use serde::Serialize;
use thiserror::Error;
use tokio::sync::oneshot;
use tracing::error;

use crate::{relays::StorageRelay, Cryptarchia};

const MAX_NUMBER_OF_BLOCKS: usize = 1000;

#[derive(Debug, Error)]
pub enum GetBlocksError {
    #[error("Storage channel dropped")]
    ChannelDropped,
    #[error("Block not found in storage for header {0:?}")]
    BlockNotFound(HeaderId),
    #[error("Failed to send storage request")]
    SendError,
    #[error("Failed to convert block")]
    ConversionError,
}

pub struct BlockProvider<Storage, State, Tx, BlobCertificate>
where
    Storage: StorageBackend,
{
    storage_relay: StorageRelay<Storage>,
    _phantom: PhantomData<State>,
    _phantom_tx: PhantomData<Tx>,
    _phantom_blob: PhantomData<BlobCertificate>,
}

impl<Storage, State, Tx, BlobCertificate> BlockProvider<Storage, State, Tx, BlobCertificate>
where
    Storage: StorageBackend + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<Tx, BlobCertificate>> + TryInto<Block<Tx, BlobCertificate>>,
    State: CryptarchiaState,
    Tx: Clone + Eq + Hash + Serialize,
    BlobCertificate: Clone + Eq + Hash + Serialize,
{
    pub fn new(storage_relay: StorageRelay<Storage>) -> Self {
        Self {
            storage_relay,
            _phantom: PhantomData,
            _phantom_tx: PhantomData,
            _phantom_blob: PhantomData,
        }
    }

    /// Returns a stream of up to MAX_NUMBER_OF_BLOCKS serialized blocks from a
    /// known block towards the target_block, in parent-to-child order. The
    /// stream yields blocks one by one and terminates early if an error is
    /// encountered.
    pub async fn send_blocks(
        &self,
        cryptarchia: &Cryptarchia<State>,
        local_tip: HeaderId,
        target_block: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: Vec<HeaderId>,
        replay_sender: oneshot::Sender<BoxStream<'static, Vec<u8>>>,
    ) -> Result<(), GetBlocksError> {
        let known_blocks = additional_blocks
            .into_iter()
            .chain([local_tip, latest_immutable_block])
            .collect::<Vec<_>>();

        let Some(start_block) = Self::max_lca(cryptarchia, target_block, known_blocks) else {
            error!("Failed to find LCA for target block {target_block:?} and known blocks");
            replay_sender
                .send(Box::pin(stream::empty::<Vec<u8>>()))
                .map_err(|_| GetBlocksError::SendError)?;
            return Ok(());
        };

        let path = Self::compute_path(cryptarchia, start_block, target_block, MAX_NUMBER_OF_BLOCKS);

        let storage_relay = self.storage_relay.clone();

        // Here we can't return a stream from storage because blocks aren't ordered by
        // their IDs in storage.
        let stream = stream::iter(path.into_iter()).then(move |id| {
            let storage = storage_relay.clone();
            async move {
                let (tx, rx) = oneshot::channel();
                storage
                    .send(StorageMsg::get_block_request(id, tx))
                    .map_err(|_| GetBlocksError::SendError)
                    .await?;

                let block = rx
                    .await
                    .map_err(|_| GetBlocksError::ChannelDropped)?
                    .ok_or(GetBlocksError::BlockNotFound(id))?;

                let block = block
                    .try_into()
                    .map_err(|_| GetBlocksError::ConversionError)?;

                let serialized_block =
                    wire::serialize(&block).map_err(|_| GetBlocksError::ConversionError)?;

                Ok(serialized_block)
            }
        });

        let stream = stream
            .take_while(|result| futures::future::ready(result.is_ok()))
            // This will never happen but forces the expected type
            .map(|result: Result<Vec<u8>, GetBlocksError>| result.unwrap_or_default());

        replay_sender
            .send(Box::pin(stream))
            .map_err(|_| GetBlocksError::SendError)?;

        Ok(())
    }

    fn max_lca(
        cryptarchia: &Cryptarchia<State>,
        target_block: HeaderId,
        known_blocks: Vec<HeaderId>,
    ) -> Option<HeaderId> {
        let branches = cryptarchia.consensus.branches();
        let target = branches.get(&target_block)?;

        known_blocks
            .iter()
            .filter_map(|known| branches.get(known).map(|b| branches.lca(b, target)))
            .max_by_key(|b| b.length())
            .map(|b| b.id())
    }

    fn compute_path(
        cryptarchia: &Cryptarchia<State>,
        start_block: HeaderId,
        target_block: HeaderId,
        limit: usize,
    ) -> VecDeque<HeaderId> {
        let mut path = VecDeque::new();
        let mut current = Some(target_block);

        while let Some(id) = current {
            if id == start_block {
                break;
            }

            if path.len() == limit {
                path.pop_back();
            }

            path.push_front(id);

            let parent = cryptarchia
                .consensus
                .branches()
                .get(&id)
                .map(|block| block.parent());

            if parent.is_none() {
                error!("Failed to find parent for block {id:?}");
                return VecDeque::new();
            }

            if Some(id) == parent {
                break;
            }

            current = parent;
        }

        path
    }
}
