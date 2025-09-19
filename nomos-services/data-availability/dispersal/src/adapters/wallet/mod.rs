pub mod mock;

use nomos_core::{
    da::BlobId,
    mantle::{
        ops::channel::{ChannelId, Ed25519PublicKey, MsgId},
        SignedMantleTx,
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
        signer: Ed25519PublicKey,
    ) -> Result<SignedMantleTx, Self::Error>;
}
