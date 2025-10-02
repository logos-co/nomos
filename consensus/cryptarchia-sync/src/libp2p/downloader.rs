use futures::stream;
use libp2p::PeerId;
use libp2p_stream::Control;
use tokio::{sync::oneshot, time, time::Duration};
use tracing::error;

use crate::{
    DownloadBlocksRequest, GetTipResponse,
    libp2p::{
        behaviour::{BlocksRequestStream, BoxedStream, TipRequestStream},
        errors::{ChainSyncError, ChainSyncErrorKind},
        messages::{DownloadBlocksResponse, RequestMessage},
        packing::unpack_from_reader,
        stream::{Stream, StreamCloser},
        utils::send_message,
    },
    messages::SerialisedBlock,
};

pub struct Downloader;

impl Downloader {
    pub async fn send_tip_request(
        peer_id: PeerId,
        control: &mut Control,
        reply_sender: oneshot::Sender<Result<GetTipResponse, ChainSyncError>>,
    ) -> Result<(TipRequestStream, StreamCloser), ChainSyncError> {
        let (mut stream, stream_closer) = Stream::open(peer_id, control).await?;

        let tip_request = RequestMessage::GetTip;
        if let Err(e) = send_message(peer_id, &mut stream, &tip_request).await {
            drop(stream);
            stream_closer.await;
            return Err(e);
        }

        let request_stream = TipRequestStream::new(peer_id, stream, reply_sender);
        Ok((request_stream, stream_closer))
    }

    pub async fn send_download_request(
        peer_id: PeerId,
        mut control: Control,
        request: DownloadBlocksRequest,
        reply_sender: oneshot::Sender<BoxedStream<Result<SerialisedBlock, ChainSyncError>>>,
    ) -> Result<(BlocksRequestStream, StreamCloser), ChainSyncError> {
        let (mut stream, stream_closer) = Stream::open(peer_id, &mut control).await?;

        let download_request = RequestMessage::DownloadBlocksRequest(request);

        if let Err(e) = send_message(peer_id, &mut stream, &download_request).await {
            drop(stream);
            stream_closer.await;
            return Err(e);
        }

        let request_stream = BlocksRequestStream::new(peer_id, stream, reply_sender);
        Ok((request_stream, stream_closer))
    }

    pub async fn receive_tip(
        request_stream: TipRequestStream,
        timeout: Duration,
    ) -> Result<(), ChainSyncError> {
        let TipRequestStream {
            mut stream,
            peer_id,
            reply_channel,
        } = request_stream;

        let response = time::timeout(
            timeout,
            unpack_from_reader::<GetTipResponse, _>(&mut stream),
        )
        .await
        .map_err(|e| {
            error!("Timeout while receiving tip from peer {}", peer_id);
            ChainSyncError::from((peer_id, e))
        })?
        .map_err(|e| ChainSyncError::from((peer_id, e)))
        .and_then(|response| match response {
            tip @ GetTipResponse::Tip { .. } => Ok(tip),
            GetTipResponse::Failure(reason) => Err(ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::RequestTipError(reason),
            }),
        });

        reply_channel.send(response).map_err(|e| ChainSyncError {
            peer: peer_id,
            kind: ChainSyncErrorKind::ChannelSendError(format!(
                "Failed to send tip response result to {peer_id:?}: {e:?}"
            )),
        })
    }

    pub async fn receive_blocks(
        request_stream: BlocksRequestStream,
        timeout: Duration,
    ) -> Result<(), ChainSyncError> {
        let stream = request_stream.stream;
        let peer_id = request_stream.peer_id;
        let reply_channel = request_stream.reply_channel;

        let stream = stream::try_unfold(stream, move |mut stream| async move {
            let response = time::timeout(timeout, unpack_from_reader(&mut stream)).await;

            match response {
                Ok(Ok(DownloadBlocksResponse::Block(block))) => Ok(Some((block, stream))),
                Ok(Ok(DownloadBlocksResponse::NoMoreBlocks)) => Ok(None),
                Ok(Ok(DownloadBlocksResponse::Failure(reason))) => Err(ChainSyncError {
                    peer: peer_id,
                    kind: ChainSyncErrorKind::RequestBlocksDownloadError(reason),
                }),
                Ok(Err(e)) => Err(ChainSyncError::from((peer_id, e))),
                Err(e) => Err(ChainSyncError::from((peer_id, e))),
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
