use futures::{future::BoxFuture, stream, stream::BoxStream, StreamExt as _};
use libp2p::{PeerId, Stream as Libp2pStream};
use libp2p_stream::Control;
use tokio::sync::mpsc::Sender;
use tracing::error;

use crate::{
    behaviour::{BlocksResponse, RequestStream, ResponseKind, TipResponse},
    errors::ChainSyncError,
    messages::{DownloadBlocksRequest, DownloadBlocksResponse, GetTipResponse, RequestMessage},
    packing::unpack_from_reader,
    utils,
    utils::{open_stream, send_message},
    ChainSyncErrorKind,
};

pub struct DownloadBlocksTask;

impl DownloadBlocksTask {
    pub async fn send_tip_request(
        peer_id: PeerId,
        control: &mut Control,
        reply_sender: Sender<BoxStream<'static, Result<ResponseKind, ChainSyncError>>>,
    ) -> Result<
        (
            RequestStream,
            Sender<BoxStream<'static, Result<ResponseKind, ChainSyncError>>>,
        ),
        ChainSyncError,
    > {
        let mut stream = open_stream(peer_id, control).await?;

        let tip_request = RequestMessage::GetTip;
        send_message(peer_id, &mut stream, &tip_request).await?;

        let request_stream = RequestStream::new(peer_id, stream, tip_request);
        Ok((request_stream, reply_sender))
    }

    pub async fn send_download_request(
        peer_id: PeerId,
        mut control: Control,
        request: DownloadBlocksRequest,
        reply_sender: Sender<BoxStream<'static, Result<ResponseKind, ChainSyncError>>>,
    ) -> Result<
        (
            RequestStream,
            Sender<BoxStream<'static, Result<ResponseKind, ChainSyncError>>>,
        ),
        ChainSyncError,
    > {
        let mut stream = open_stream(peer_id, &mut control).await?;

        let download_request = RequestMessage::DownloadBlocksRequest(request);
        send_message(peer_id, &mut stream, &download_request).await?;

        let request_stream = RequestStream::new(peer_id, stream, download_request);
        Ok((request_stream, reply_sender))
    }

    pub fn receive_tip(
        peer_id: PeerId,
        mut stream: Libp2pStream,
        reply_tx: Sender<BoxStream<'static, Result<ResponseKind, ChainSyncError>>>,
    ) -> BoxFuture<'static, Result<(), ChainSyncError>> {
        Box::pin(async move {
            let response_result = unpack_from_reader::<GetTipResponse, _>(&mut stream)
                .await
                .map_err(|e| ChainSyncError::from((peer_id, e)));

            let tip_stream: BoxStream<'static, Result<ResponseKind, ChainSyncError>> =
                match response_result {
                    Ok(response) => {
                        let tip_response =
                            ResponseKind::Tip(TipResponse::Tip((peer_id, response.tip)));
                        stream::iter(vec![Ok(tip_response)])
                    }
                    .boxed(),
                    Err(e) => {
                        error!("Failed to get tip for peer {}: {}", peer_id, e);
                        utils::close_stream(peer_id, stream).await?;

                        stream::iter(vec![Ok(ResponseKind::Tip(TipResponse::NetworkError(e)))])
                            .boxed()
                    }
                };

            reply_tx.send(tip_stream).await.map_err(|e| ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::ChannelSendError(format!(
                    "Failed to send tip stream: {e}",
                )),
            })
        })
    }

    pub fn receive_blocks(
        peer_id: PeerId,
        libp2p_stream: Libp2pStream,
        reply_tx: Sender<BoxStream<'static, Result<ResponseKind, ChainSyncError>>>,
    ) -> BoxFuture<'static, Result<(), ChainSyncError>> {
        Box::pin(async move {
            let stream = Box::pin(stream::try_unfold(
                libp2p_stream,
                move |mut stream| async move {
                    match unpack_from_reader::<DownloadBlocksResponse, _>(&mut stream).await {
                        Ok(DownloadBlocksResponse::Block(block)) => Ok(Some((
                            ResponseKind::Blocks(BlocksResponse::Block((peer_id, block))),
                            stream,
                        ))),
                        Ok(DownloadBlocksResponse::NoMoreBlocks) => {
                            utils::close_stream(peer_id, stream).await?;
                            Ok(None)
                        }
                        Err(e) => {
                            error!("Failed to receive blocks from peer {}: {}", peer_id, e);
                            utils::close_stream(peer_id, stream).await?;

                            Err(ChainSyncError::from((peer_id, e)))
                        }
                    }
                },
            ));

            reply_tx.send(stream).await.map_err(|e| ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::ChannelSendError(format!(
                    "Failed to send blocks stream: {e}"
                )),
            })
        })
    }
}
