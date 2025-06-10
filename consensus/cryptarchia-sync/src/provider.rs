use bytes::Bytes;
use futures::{channel::oneshot, stream::BoxStream, StreamExt as _, TryStreamExt as _};
use libp2p::{PeerId, Stream as Libp2pStream};
use tokio::sync::mpsc;

use crate::{
    behaviour::RequestStream,
    errors::{ChainSyncError, ChainSyncErrorKind},
    messages::{DownloadBlocksResponse, GetTipResponse, RequestMessage, SerialisedBlock},
    packing::unpack_from_reader,
    utils::send_message,
};

pub const MAX_ADDITIONAL_BLOCKS: usize = 5;

pub struct ProvideBlocksTask;

impl ProvideBlocksTask {
    pub async fn process_request(
        peer_id: PeerId,
        mut stream: Libp2pStream,
    ) -> Result<RequestStream, ChainSyncError> {
        let request: RequestMessage = unpack_from_reader(&mut stream)
            .await
            .map_err(|e| ChainSyncError::from((peer_id, e)))?;

        Ok(RequestStream::new(peer_id, stream, request))
    }

    pub async fn provide_tip(
        reply_rcv: oneshot::Receiver<BoxStream<'static, Result<Bytes, ChainSyncError>>>,
        peer_id: PeerId,
        mut libp2p_stream: Libp2pStream,
    ) -> Result<(), ChainSyncError> {
        let mut stream = reply_rcv.await.map_err(|_| ChainSyncError {
            peer: peer_id,
            kind: ChainSyncErrorKind::ChannelReceiveError(
                "Failed to receive blocks stream".to_owned(),
            ),
        })?;

        let tip = stream.next().await.ok_or_else(|| ChainSyncError {
            peer: peer_id,
            kind: ChainSyncErrorKind::ChannelReceiveError("No tip received".to_owned()),
        })??;

        let response = GetTipResponse { tip };
        send_message(peer_id, &mut libp2p_stream, &response).await?;

        Ok(())
    }

    pub async fn provide_blocks(
        mut reply_rcv: mpsc::Receiver<BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>>,
        peer_id: PeerId,
        mut libp2p_stream: Libp2pStream,
    ) -> Result<(), ChainSyncError> {
        let stream = reply_rcv.recv().await.ok_or_else(|| ChainSyncError {
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
