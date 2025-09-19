pub mod mock;

use nomos_core::mantle::{ops::channel::ChannelId, SignedMantleTx};

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
