use futures::{stream::BoxStream, TryStreamExt as _};
use libp2p::{PeerId, Stream as Libp2pStream};
use tokio::sync::{mpsc, oneshot};

use crate::{
    errors::{ChainSyncError, ChainSyncErrorKind},
    messages::{
        DownloadBlocksResponse, GetTipResponse, RequestMessage, SerialisedBlock, SerialisedHeaderId,
    },
    packing::unpack_from_reader,
    utils::send_message,
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
        reply_receiver: oneshot::Receiver<SerialisedHeaderId>,
        peer_id: PeerId,
        mut libp2p_stream: Libp2pStream,
    ) -> Result<(), ChainSyncError> {
        let tip = reply_receiver.await.map_err(|_| ChainSyncError {
            peer: peer_id,
            kind: ChainSyncErrorKind::ChannelReceiveError(
                "Failed to receive blocks stream".to_owned(),
            ),
        })?;

        let response = GetTipResponse { tip };
        send_message(peer_id, &mut libp2p_stream, &response).await?;

        Ok(())
    }

    pub async fn provide_blocks(
        mut reply_receiver: mpsc::Receiver<
            BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>,
        >,
        peer_id: PeerId,
        mut libp2p_stream: Libp2pStream,
    ) -> Result<(), ChainSyncError> {
        let stream = reply_receiver.recv().await.ok_or_else(|| ChainSyncError {
            peer: peer_id,
            kind: ChainSyncErrorKind::ChannelReceiveError(
                "Failed to receive blocks stream".to_owned(),
            ),
        })?;

        stream
            .try_fold(&mut libp2p_stream, |stream, block| async move {
                let message = DownloadBlocksResponse::Block(block);
                send_message(peer_id, stream, &message).await?;
                Ok(stream)
            })
            .await?;

        let request = DownloadBlocksResponse::NoMoreBlocks;
        send_message(peer_id, &mut libp2p_stream, &request).await?;

        Ok(())
    }
}
