use std::{
    collections::{HashSet, VecDeque},
    fmt::Debug,
    hash::Hash,
};

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
    #[error("Invalid state: {0}")]
    InvalidState(String),
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
    ) where
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
            max_lca(cryptarchia.consensus.branches(), target_block, known_blocks)
        else {
            Self::send_error(
                "Failed to find LCA for target block and known blocks".to_owned(),
                reply_sender,
            )
            .await;

            return;
        };

        info!("Starting to send blocks from {start_block:?} to {target_block:?}");

        let Ok(path) = compute_path(
            cryptarchia.consensus.branches(),
            start_block,
            target_block,
            MAX_NUMBER_OF_BLOCKS,
        ) else {
            Self::send_error(
                "Failed to compute path from start block to target block".to_owned(),
                reply_sender,
            )
            .await;

            return;
        };

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

fn max_lca<Id>(branches: &Branches<Id>, target_block: Id, known_blocks: &HashSet<Id>) -> Option<Id>
where
    Id: Hash + Eq + Copy + Debug,
{
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

fn compute_path<Id>(
    branches: &Branches<Id>,
    start_block: Id,
    target_block: Id,
    limit: usize,
) -> Result<VecDeque<Id>, GetBlocksError>
where
    Id: Copy + Eq + Hash + Debug,
{
    let mut path = VecDeque::new();

    let mut current = target_block;
    loop {
        path.push_front(current);

        if path.len() > limit {
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

    use cryptarchia_engine::{Boostrapping, Config, Slot};

    use super::*;

    #[test]
    fn test_compute_path() {
        let mut cryptarchia = new_cryptarchia();

        cryptarchia = cryptarchia
            .receive_block([1; 32], [0; 32], Slot::from(1))
            .expect("Failed to add block");

        cryptarchia = cryptarchia
            .receive_block([2; 32], [1; 32], Slot::from(2))
            .expect("Failed to add block");

        let branches = cryptarchia.branches();

        let start_block = [1; 32];
        let target_block = [2; 32];

        let path = compute_path(branches, start_block, target_block, MAX_NUMBER_OF_BLOCKS).unwrap();

        assert_eq!(path.len(), 2);
        assert_eq!(path.front().unwrap(), &start_block);
        assert_eq!(path.back().unwrap(), &target_block);
    }

    #[test]
    fn test_compute_path_limit_bounds() {
        let mut cryptarchia = new_cryptarchia();

        cryptarchia = cryptarchia
            .receive_block([1; 32], [0; 32], Slot::from(1))
            .expect("Failed to add block");

        cryptarchia = cryptarchia
            .receive_block([2; 32], [1; 32], Slot::from(2))
            .expect("Failed to add block");

        cryptarchia = cryptarchia
            .receive_block([3; 32], [2; 32], Slot::from(3))
            .expect("Failed to add block");

        let branches = cryptarchia.branches();

        let start_block = [1; 32];
        let target_block = [3; 32];
        let last_block_in_computed_path = [2; 32];

        let limit = 2;

        let path = compute_path(branches, start_block, target_block, limit).unwrap();

        assert_eq!(path.len(), limit);
        assert_eq!(path.front().unwrap(), &start_block);
        assert_eq!(path.back().unwrap(), &last_block_in_computed_path);
    }

    #[test]
    fn test_compute_path_unreachable_start() {
        let cryptarchia = new_cryptarchia();

        let start_block = [1; 32];
        let target_block = [2; 32];

        let branches = cryptarchia.branches();

        let path = compute_path(branches, start_block, target_block, MAX_NUMBER_OF_BLOCKS);

        assert!(matches!(
            path,
            Err(GetBlocksError::InvalidState(ref msg)) if msg.contains("Couldn't reach start_block")
        ));
    }

    #[test]
    fn test_compute_path_hits_genesis() {
        let mut cryptarchia = new_cryptarchia();

        cryptarchia = cryptarchia
            .receive_block([3; 32], [0; 32], Slot::from(1))
            .expect("Failed to add block");

        let branches = cryptarchia.branches();

        let start_block_not_existing = [2; 32];
        let target_block = [3; 32];

        let path = compute_path(
            branches,
            start_block_not_existing,
            target_block,
            MAX_NUMBER_OF_BLOCKS,
        );

        assert!(matches!(
            path,
            Err(GetBlocksError::InvalidState(ref msg)) if msg.contains("Genesis block reached")
        ));
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
