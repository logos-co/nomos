pub mod mock;

use nomos_core::{
    da::BlobId,
    mantle::{
        SignedMantleTx,
        ops::channel::{ChannelId, Ed25519PublicKey, MsgId},
        tx_builder::MantleTxBuilder,
    },
};

#[async_trait::async_trait]
pub trait DaWalletAdapter {
    type Error;

    // TODO: Pass relay when wallet service is defined.
    fn new() -> Self;

    fn blob_tx(
        &self,
        tx_builder: MantleTxBuilder,
        channel_id: ChannelId,
        parent_msg_id: MsgId,
        blob_id: BlobId,
        blob_size: usize,
        signer: Ed25519PublicKey,
    ) -> Result<SignedMantleTx, Self::Error>;
}
