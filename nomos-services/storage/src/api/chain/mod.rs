pub mod requests;

use std::{collections::BTreeMap, error::Error, num::NonZeroUsize, ops::RangeInclusive};

use async_trait::async_trait;
use cryptarchia_engine::Slot;
use nomos_core::header::HeaderId;

#[async_trait]
pub trait StorageChainApi {
    type Error: Error + Send + Sync + 'static;
    type Block: Send + Sync;

    async fn get_block(&mut self, header_id: HeaderId) -> Result<Option<Self::Block>, Self::Error>;

    async fn store_block(
        &mut self,
        header_id: HeaderId,
        block: Self::Block,
    ) -> Result<(), Self::Error>;

    async fn remove_block(
        &mut self,
        header_id: HeaderId,
    ) -> Result<Option<Self::Block>, Self::Error>;

    async fn store_immutable_block_ids(
        &mut self,
        ids: BTreeMap<Slot, HeaderId>,
    ) -> Result<(), Self::Error>;

    async fn get_immutable_block_id(&mut self, slot: Slot)
    -> Result<Option<HeaderId>, Self::Error>;

    async fn scan_immutable_block_ids(
        &mut self,
        slot_range: RangeInclusive<Slot>,
        limit: NonZeroUsize,
    ) -> Result<Vec<HeaderId>, Self::Error>;
}
