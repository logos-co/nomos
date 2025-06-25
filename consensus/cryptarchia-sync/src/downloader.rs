use futures::stream;
use libp2p::PeerId;
use libp2p_stream::Control;
use nomos_core::header::HeaderId;
use tokio::sync::oneshot;
use tracing::error;

use crate::{
    behaviour::{BlocksRequestStream, BoxedStream, TipRequestStream},
    errors::ChainSyncError,
    messages::{DownloadBlocksRequest, DownloadBlocksResponse, GetTipResponse, RequestMessage},
    packing::unpack_from_reader,
    utils,
    utils::{open_stream, send_message},
    ChainSyncErrorKind, SerialisedBlock,
};

pub struct Downloader;

impl Downloader {
    pub async fn send_tip_request(
        peer_id: PeerId,
        control: &mut Control,
        reply_sender: oneshot::Sender<Result<HeaderId, ChainSyncError>>,
    ) -> Result<TipRequestStream, ChainSyncError> {
        let mut stream = open_stream(peer_id, control).await?;

        let tip_request = RequestMessage::GetTip;
        send_message(peer_id, &mut stream, &tip_request).await?;

        let request_stream = TipRequestStream::new(peer_id, stream, reply_sender);
        Ok(request_stream)
    }

    pub async fn send_download_request(
        peer_id: PeerId,
        mut control: Control,
        request: DownloadBlocksRequest,
        reply_sender: oneshot::Sender<BoxedStream<Result<SerialisedBlock, ChainSyncError>>>,
    ) -> Result<BlocksRequestStream, ChainSyncError> {
        let mut stream = open_stream(peer_id, &mut control).await?;

        let download_request = RequestMessage::DownloadBlocksRequest(request);

        send_message(peer_id, &mut stream, &download_request).await?;

        let request_stream = BlocksRequestStream::new(peer_id, stream, reply_sender);
        Ok(request_stream)
    }

    pub async fn receive_tip(request_stream: TipRequestStream) -> Result<(), ChainSyncError> {
        let TipRequestStream {
            mut stream,
            peer_id,
            reply_channel,
        } = request_stream;

        let tip_response = match unpack_from_reader::<GetTipResponse, _>(&mut stream).await {
            Ok(GetTipResponse { tip }) => Ok(tip),
            Err(e) => {
                error!("Failed to receive tip from peer {peer_id}: {e}");
                Err(ChainSyncError::from((peer_id, e)))
            }
        };

        if let Err(e) = reply_channel.send(tip_response) {
            error!("Failed to send tip response to peer {peer_id}: {e:?}");
        }

        utils::close_stream(peer_id, stream).await
    }
    pub async fn receive_blocks(request_stream: BlocksRequestStream) -> Result<(), ChainSyncError> {
        let libp2p_stream = request_stream.stream;
        let peer_id = request_stream.peer_id;
        let reply_channel = request_stream.reply_channel;

        let stream = stream::try_unfold(libp2p_stream, move |mut stream| async move {
            match unpack_from_reader::<DownloadBlocksResponse, _>(&mut stream).await {
                Ok(DownloadBlocksResponse::Block(block)) => Ok(Some((block, stream))),
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
        });

        let boxed_stream: BoxedStream<_> = Box::new(Box::pin(stream));
        reply_channel
            .send(boxed_stream)
            .map_err(|_| ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::ChannelSendError(
                    "Failed to send blocks stream".to_string(),
                ),
            })
    }
}
