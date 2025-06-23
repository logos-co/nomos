use std::collections::{HashSet, VecDeque};

use bytes::Bytes;
use cryptarchia_engine::{Branch, Branches, CryptarchiaState};
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
    #[error("Failed to send to channel: {0}")]
    SendError(String),
    #[error("Failed to convert block")]
    ConversionError,
}

pub struct BlockProvider<Storage>
where
    Storage: StorageBackend,
{
    storage_relay: StorageRelay<Storage>,
}
impl<Storage: StorageBackend + 'static> BlockProvider<Storage> {
    pub const fn new(storage_relay: StorageRelay<Storage>) -> Self {
        Self { storage_relay }
    }

    /// Returns a stream of serialized blocks up to `MAX_NUMBER_OF_BLOCKS` from
    /// a known block towards the `target_block`, in parent-to-child order.
    /// The stream yields blocks one by one and terminates early if an error
    /// is encountered.
    pub async fn send_blocks<State, Tx, BlobCertificate>(
        &self,
        cryptarchia: &Cryptarchia<State>,
        target_block: HeaderId,
        known_blocks: &HashSet<HeaderId>,
        reply_sender: Sender<BoxStream<'static, Result<Bytes, DynError>>>,
    ) -> Result<(), GetBlocksError>
    where
        State: CryptarchiaState + Send + Sync + 'static,
        <Storage as StorageChainApi>::Block: TryInto<Block<Tx, BlobCertificate>>,
        Tx: Serialize + Clone + Eq + 'static,
        BlobCertificate: Serialize + Clone + Eq + 'static,
    {
        info!(
            "Requesting blocks with inputs:
            target_block={target_block:?},
            known_blocks={known_blocks:?},"
        );

        let Some(start_block) =
            Self::max_lca(cryptarchia.consensus.branches(), target_block, known_blocks)
        else {
            error!("Failed to find LCA for target block and known blocks");

            let stream = stream::once(async move { Err(DynError::from("LCA not found")) });

            reply_sender
                .send(Box::pin(stream))
                .await
                .map_err(|_| GetBlocksError::SendError("Failed to send error stream".to_owned()))?;

            return Ok(());
        };

        info!("Starting to send blocks from {start_block:?} to {target_block:?}");

        let path = Self::compute_path(
            cryptarchia.consensus.branches(),
            start_block,
            target_block,
            MAX_NUMBER_OF_BLOCKS,
        );

        let storage_relay = self.storage_relay.clone();

        // Here we can't return a stream from storage because blocks aren't ordered by
        // their IDs in storage.
        let stream = stream::iter(path)
            .then(move |id| {
                let storage = storage_relay.clone();
                async move {
                    let (tx, rx) = oneshot::channel();
                    if let Err((e, _)) = storage.send(StorageMsg::get_block_request(id, tx)).await {
                        error!("Failed to send block request for {id:?}: {e}");
                    }

                    let block = rx
                        .await
                        .map_err(|_| GetBlocksError::ChannelDropped)?
                        .ok_or(GetBlocksError::BlockNotFound(id))?;

                    let block = block
                        .try_into()
                        .map_err(|_| GetBlocksError::ConversionError)?;

                    let serialized_block = wire::serialize(&block)
                        .map_err(|_| GetBlocksError::ConversionError)?
                        .into();

                    Ok(serialized_block)
                }
            })
            .map_err(|e: GetBlocksError| {
                error!("Error processing block: {e}");
                DynError::from(e)
            })
            .take_while(|result| future::ready(result.is_ok()));

        if let Err(e) = reply_sender.send(Box::pin(stream)).await {
            error!("Failed to send blocks stream: {e}");
        }

        Ok(())
    }

    fn max_lca(
        branches: &Branches<HeaderId>,
        target_block: HeaderId,
        known_blocks: &HashSet<HeaderId>,
    ) -> Option<HeaderId> {
        let target_branch = branches.get(&target_block)?;

        known_blocks
            .iter()
            .filter_map(|known| {
                branches
                    .get(known)
                    .map(|known_branch| branches.lca(known_branch, target_branch))
            })
            .max_by_key(Branch::length)
            .map(|b| b.id())
    }

    fn compute_path(
        branches: &Branches<HeaderId>,
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

            let parent = branches.get(&id).map(Branch::parent);

            // If the parent the same as the current block, we have reached genesis.
            if Some(id) == parent {
                break;
            }

            current = parent;
        }

        path
    }
}
