pub mod mock;

use nomos_core::mantle::{
    SignedMantleTx,
    ops::channel::{ChannelId, MsgId},
};

pub struct DispersalStorageError;

pub trait DispersalStorageAdapter {
    fn new() -> Self;
    fn last_tx_id(&self, channel_id: &ChannelId) -> MsgId;
    fn store_tx(
        &mut self,
        channel_id: ChannelId,
        tx: SignedMantleTx,
    ) -> Result<(), DispersalStorageError>;
}
