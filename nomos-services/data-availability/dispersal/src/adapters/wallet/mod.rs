pub mod mock;

use nomos_core::{
    da::BlobId,
    mantle::{
        SignedMantleTx,
        ops::channel::{ChannelId, MsgId},
    },
};

#[async_trait::async_trait]
pub trait DaWalletAdapter {
    type Error;

    // TODO: Pass relay when wallet service is defined.
    fn new() -> Self;

    fn blob_tx(
        &self,
        channel_id: ChannelId,
        parent_msg_id: MsgId,
        blob_id: BlobId,
        blob_size: usize,
    ) -> Result<SignedMantleTx, Self::Error>;
}
