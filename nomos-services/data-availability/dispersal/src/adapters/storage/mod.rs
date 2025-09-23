pub mod mock;

use nomos_core::mantle::{SignedMantleTx, ops::channel::ChannelId};

pub struct DispersalStorageError;

pub trait DispersalStorageAdapter {
    fn new() -> Self;
    fn last_tx(&self, channel_id: &ChannelId) -> Option<SignedMantleTx>;
    fn store_tx(
        &mut self,
        channel_id: ChannelId,
        tx: SignedMantleTx,
    ) -> Result<(), DispersalStorageError>;
}
