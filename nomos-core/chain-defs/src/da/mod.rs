pub mod blob;

use blob::Share;

use crate::{
    block::BlockNumber,
    mantle::{
        SignedMantleTx,
        ops::channel::{ChannelId, Ed25519PublicKey, MsgId},
    },
    sdp,
};

pub type BlobId = [u8; 32];

pub trait DaEncoder {
    type EncodedData;
    type Error;

    fn encode(&self, b: &[u8]) -> Result<Self::EncodedData, Self::Error>;
}

pub trait DaVerifier {
    type DaShare: Share;
    type Error;

    fn verify(
        &self,
        commitments: &<Self::DaShare as Share>::SharesCommitments,
        light_share: &<Self::DaShare as Share>::LightShare,
    ) -> Result<(), Self::Error>;
}

#[async_trait::async_trait]
pub trait DaDispersal {
    type EncodedData;
    type Error;

    async fn disperse_shares(&self, encoded_data: Self::EncodedData) -> Result<(), Self::Error>;
    async fn disperse_tx(
        &self,
        channel_id: ChannelId,
        parent_msg_id: MsgId,
        blob_id: BlobId,
        num_columns: usize,
        original_size: usize,
        signer: Ed25519PublicKey,
    ) -> Result<SignedMantleTx, Self::Error>;
}

#[must_use]
pub const fn availability_window_size(service_params: &sdp::ServiceParameters) -> BlockNumber {
    service_params
        .session_duration
        .checked_mul(2)
        .expect("session duration should not overflow")
}
