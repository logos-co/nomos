use std::{
    collections::{HashSet, VecDeque},
    hash::Hash,
    marker::PhantomData,
};

use cryptarchia_engine::CryptarchiaState;
use futures::{StreamExt, TryFutureExt};
use nomos_core::{block::Block, header::HeaderId};
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend, StorageMsg};
use thiserror::Error;

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
    Storage: StorageBackend,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<Tx, BlobCertificate>> + TryInto<Block<Tx, BlobCertificate>>,
    State: CryptarchiaState,
    Tx: Clone + Eq + Hash,
    BlobCertificate: Clone + Eq + Hash,
{
    pub fn new(storage_relay: StorageRelay<Storage>) -> Self {
        Self {
            storage_relay,
            _phantom: PhantomData,
            _phantom_tx: PhantomData,
            _phantom_blob: PhantomData,
        }
    }

    /// Returns up to MAX_NUMBER_OF_BLOCKS blocks from a known block towards the
    /// tip, in parent-to-child order.
    ///
    /// ## Performance
    /// - Assumes approximately 1 million blocks per year.
    /// - Worst case (from genesis): ~100ms
    pub async fn get_blocks(
        &self,
        cryptarchia: &Cryptarchia<State>,
        target_block: Option<HeaderId>,
        known_blocks: Vec<HeaderId>,
    ) -> Result<Vec<Block<Tx, BlobCertificate>>, GetBlocksError> {
        let branches = cryptarchia.consensus.branches();
        let tip = target_block.unwrap_or_else(|| cryptarchia.tip());
        let known_set: HashSet<_> = known_blocks.into_iter().collect();

        let mut path = VecDeque::new();
        let mut current = Some(tip);

        while let Some(id) = current {
            if known_set.contains(&id) {
                break;
            }
            if path.len() == MAX_NUMBER_OF_BLOCKS {
                path.pop_back();
            }
            path.push_front(id);
            current = branches.get(&id).and_then(|b| Some(b.parent()));
        }

        let mut path = path.into_iter();
        let storage = self.storage_relay.clone();

        let mut blocks = Vec::with_capacity(path.len());

        for id in path {
            let (tx, rx) = tokio::sync::oneshot::channel();
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
                .map_err(|e| GetBlocksError::ConversionError)?;

            blocks.push(block);
        }

        Ok(blocks)
    }
}
