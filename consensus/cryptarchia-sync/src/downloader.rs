use std::future::Future;

use futures::{stream, stream::BoxStream, StreamExt as _};
use libp2p::{PeerId, Stream as Libp2pStream};
use libp2p_stream::Control;
use tokio::sync::{mpsc::Sender, oneshot};
use tracing::error;

use crate::{
    behaviour::{BlocksResponse, ReplyChannel, RequestStream, TipResponse},
    errors::ChainSyncError,
    messages::{DownloadBlocksRequest, DownloadBlocksResponse, GetTipResponse, RequestMessage},
    packing::unpack_from_reader,
    utils,
    utils::{open_stream, send_message},
    ChainSyncErrorKind,
};

pub struct Downloader;

impl Downloader {
    pub async fn send_tip_request(
        peer_id: PeerId,
        control: &mut Control,
        reply_sender: oneshot::Sender<TipResponse>,
    ) -> Result<RequestStream, ChainSyncError> {
        let mut stream = open_stream(peer_id, control).await?;

        let tip_request = RequestMessage::GetTip;
        send_message(peer_id, &mut stream, &tip_request).await?;

        let request_stream = RequestStream::new(
            peer_id,
            stream,
            tip_request,
            ReplyChannel::Tip(reply_sender),
        );

        Ok(request_stream)
    }

    pub async fn send_download_request(
        peer_id: PeerId,
        mut control: Control,
        request: DownloadBlocksRequest,
        reply_sender: Sender<BoxStream<'static, Result<BlocksResponse, ChainSyncError>>>,
    ) -> Result<RequestStream, ChainSyncError> {
        let mut stream = open_stream(peer_id, &mut control).await?;

        let download_request = RequestMessage::DownloadBlocksRequest(request);
        send_message(peer_id, &mut stream, &download_request).await?;

        let request_stream = RequestStream::new(
            peer_id,
            stream,
            download_request,
            ReplyChannel::Blocks(reply_sender.clone()),
        );

        Ok(request_stream)
    }

    pub fn receive_tip(
        request_stream: RequestStream,
    ) -> impl Future<Output = Result<(), ChainSyncError>> + 'static {
        async move {
            let RequestStream {
                mut stream,
                peer_id,
                reply_channel,
                ..
            } = request_stream;

            let reply_tx = if let ReplyChannel::Tip(tx) = reply_channel {
                tx
            } else {
                return Err(ChainSyncError {
                    peer: peer_id,
                    kind: ChainSyncErrorKind::ChannelSendError(
                        "Invalid reply channel for tip".to_string(),
                    ),
                });
            };

            let tip_response = match unpack_from_reader::<GetTipResponse, _>(&mut stream).await {
                Ok(GetTipResponse { tip }) => TipResponse::Tip((peer_id, tip)),
                Err(e) => {
                    error!("Failed to receive tip from peer {peer_id}: {e}");
                    TipResponse::NetworkError(ChainSyncError {
                        peer: peer_id,
                        kind: e.into(),
                    })
                }
            };

            if let Err(e) = reply_tx.send(tip_response) {
                error!("Failed to send tip response to peer {peer_id}: {e:?}");
            }

            utils::close_stream(peer_id, stream).await
        }
    }
    pub fn receive_blocks(
        request_stream: RequestStream,
    ) -> impl Future<Output = Result<(), ChainSyncError>> + 'static {
        async move {
            let libp2p_stream = request_stream.stream;
            let peer_id = request_stream.peer_id;
            let ReplyChannel::Blocks(reply_tx) = request_stream.reply_channel else {
                return Err(ChainSyncError {
                    peer: peer_id,
                    kind: ChainSyncErrorKind::ChannelSendError(
                        "Invalid reply channel for blocks".to_owned(),
                    ),
                });
            };

            let stream = Box::pin(stream::try_unfold(
                libp2p_stream,
                move |mut stream| async move {
                    match unpack_from_reader::<DownloadBlocksResponse, _>(&mut stream).await {
                        Ok(DownloadBlocksResponse::Block(block)) => {
                            Ok(Some((BlocksResponse::Block((peer_id, block)), stream)))
                        }
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
        }
    }
}
