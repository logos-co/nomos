use futures::{stream::BoxStream, TryStreamExt as _};
use libp2p::{PeerId, Stream as Libp2pStream};
use tokio::sync::mpsc;

use crate::{
    libp2p::{
        errors::{ChainSyncError, ChainSyncErrorKind, DynError},
        packing::unpack_from_reader,
        utils::{close_stream, send_message},
    },
    messages::{DownloadBlocksResponse, GetTipResponse, RequestMessage, SerialisedBlock},
};

pub const MAX_ADDITIONAL_BLOCKS: usize = 5;

pub struct Provider;

pub type ReceivingRequestStream = (PeerId, Libp2pStream, RequestMessage);

impl Provider {
    pub async fn process_request(
        peer_id: PeerId,
        mut stream: Libp2pStream,
    ) -> Result<ReceivingRequestStream, ChainSyncError> {
        let request: RequestMessage = unpack_from_reader(&mut stream)
            .await
            .map_err(|e| ChainSyncError::from((peer_id, e)))?;

        Ok((peer_id, stream, request))
    }

    pub async fn provide_tip(
        mut reply_receiver: mpsc::Receiver<GetTipResponse>,
        peer_id: PeerId,
        mut libp2p_stream: Libp2pStream,
    ) -> Result<(), ChainSyncError> {
        let response = reply_receiver.recv().await.ok_or_else(|| ChainSyncError {
            peer: peer_id,
            kind: ChainSyncErrorKind::ChannelReceiveError(
                "Failed to receive tip from channel".to_owned(),
            ),
        })?;

        let send_result = send_message(peer_id, &mut libp2p_stream, &response).await;

        let _ = close_stream(peer_id, libp2p_stream).await;

        send_result
    }

    pub async fn provide_blocks(
        mut reply_receiver: mpsc::Receiver<BoxStream<'static, Result<SerialisedBlock, DynError>>>,
        peer_id: PeerId,
        mut libp2p_stream: Libp2pStream,
    ) -> Result<(), ChainSyncError> {
        let Some(stream) = reply_receiver.recv().await else {
            let _ = close_stream(peer_id, libp2p_stream).await;

            return Err(ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::ChannelReceiveError(
                    "Failed to receive blocks stream from channel".to_owned(),
                ),
            });
        };

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

        let final_result = match result {
            Ok(_) => {
                send_message(
                    peer_id,
                    &mut libp2p_stream,
                    &DownloadBlocksResponse::NoMoreBlocks,
                )
                .await
            }
            Err(e) => Err(e),
        };

        let _ = close_stream(peer_id, libp2p_stream).await;

        final_result
    }
}
