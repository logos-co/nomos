use futures::{TryStreamExt as _, stream::BoxStream};
use libp2p::PeerId;
use tokio::sync::mpsc;
use tracing::error;

use crate::{
    BlocksResponse, DynError, ProviderResponse, TipResponse,
    libp2p::{
        errors::{ChainSyncError, ChainSyncErrorKind},
        messages::{DownloadBlocksResponse, RequestMessage},
        packing::unpack_from_reader,
        stream::Stream,
        utils::send_message,
    },
    messages::{GetTipResponse, SerialisedBlock},
};

pub const MAX_ADDITIONAL_BLOCKS: usize = 5;

pub struct Provider;

pub type ReceivingRequestStream = (PeerId, Stream, RequestMessage);

impl Provider {
    pub async fn process_request(
        peer_id: PeerId,
        mut stream: Stream,
    ) -> Result<ReceivingRequestStream, ChainSyncError> {
        let request: RequestMessage = unpack_from_reader(&mut stream)
            .await
            .map_err(|e| ChainSyncError::from((peer_id, e)))?;

        Ok((peer_id, stream, request))
    }

    pub async fn provide_tip(
        mut reply_receiver: mpsc::Receiver<TipResponse>,
        peer_id: PeerId,
        mut stream: Stream,
    ) -> Result<(), ChainSyncError> {
        let response = reply_receiver.recv().await.ok_or_else(|| ChainSyncError {
            peer: peer_id,
            kind: ChainSyncErrorKind::ChannelReceiveError(
                "Failed to receive tip from channel".to_owned(),
            ),
        })?;

        match response {
            ProviderResponse::Available(tip) => send_message(peer_id, &mut stream, &tip).await,
            ProviderResponse::Unavailable { reason } => {
                let response = GetTipResponse::Failure(reason);
                send_message(peer_id, &mut stream, &response).await
            }
        }
    }

    pub async fn provide_blocks(
        mut reply_receiver: mpsc::Receiver<BlocksResponse>,
        peer_id: PeerId,
        mut libp2p_stream: Stream,
    ) -> Result<(), ChainSyncError> {
        let response = reply_receiver.recv().await.ok_or_else(|| ChainSyncError {
            peer: peer_id,
            kind: ChainSyncErrorKind::ChannelReceiveError(
                "Failed to receive blocks stream from channel".to_owned(),
            ),
        })?;

        match response {
            ProviderResponse::Available(stream) => {
                Self::send_blocks(peer_id, stream, libp2p_stream).await
            }
            ProviderResponse::Unavailable { reason } => {
                let response = DownloadBlocksResponse::Failure(reason);
                send_message(peer_id, &mut libp2p_stream, &response).await?;
                Ok(())
            }
        }
    }

    async fn send_blocks(
        peer_id: PeerId,
        stream: BoxStream<'static, Result<SerialisedBlock, DynError>>,
        mut libp2p_stream: Stream,
    ) -> Result<(), ChainSyncError> {
        let result = stream
            .map_err(|e| ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::ReceivingBlocksError(format!(
                    "Failed to receive block from stream: {e}"
                )),
            })
            .try_fold(&mut libp2p_stream, |stream, block| async move {
                let message = DownloadBlocksResponse::Block(block);
                send_message(peer_id, stream, &message).await?;
                Ok(stream)
            })
            .await;

        match result {
            Ok(_) => {
                let message = DownloadBlocksResponse::NoMoreBlocks;
                send_message(peer_id, &mut libp2p_stream, &message).await
            }
            Err(e) => {
                error!("Failed to send blocks to peer {}: {}", peer_id, e);
                let message = DownloadBlocksResponse::Failure(e.to_string());
                let _ = send_message(peer_id, &mut libp2p_stream, &message).await;
                Err(e)
            }
        }
    }
}
