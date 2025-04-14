use futures::{future::BoxFuture, AsyncWriteExt, StreamExt};
use libp2p::Stream;
use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    behaviour::{BehaviourSyncReply, IncomingSyncRequest, RequestKind, SyncError},
    messages::{SyncRequest, SyncResponse},
    sync_utils,
};

/// Reads peer request from the stream.
pub fn read_request_from_stream(
    mut stream: Stream,
) -> BoxFuture<'static, Result<IncomingSyncRequest, SyncError>> {
    Box::pin(async move {
        let command: SyncRequest = sync_utils::receive_data(&mut stream).await?;

        let kind = match command {
            SyncRequest::Blocks { direction } => RequestKind::Sync { direction },
            SyncRequest::TipSlot => RequestKind::TipSlot,
        };

        let (response_sender, response_receiver) = channel(10);
        Ok(IncomingSyncRequest {
            stream,
            kind,
            response_sender,
            response_stream: ReceiverStream::new(response_receiver),
        })
    })
}

/// Sends the response to the peer.
pub fn send_response_to_peer(
    mut req: IncomingSyncRequest,
) -> BoxFuture<'static, Result<(), SyncError>> {
    Box::pin(async move {
        let mut response_stream = req.response_stream;

        while let Some(service_response) = response_stream.next().await {
            match service_response {
                BehaviourSyncReply::Block(block) => {
                    let message = SyncResponse::Block(block);
                    sync_utils::send_data(&mut req.stream, &message).await?;
                }
                BehaviourSyncReply::TipSlot(tip) => {
                    let message = SyncResponse::TipSlot(tip);
                    sync_utils::send_data(&mut req.stream, &message).await?;
                }
            }
            req.stream.flush().await?;
        }

        sync_utils::send_data(&mut req.stream, &SyncResponse::End).await?;

        req.stream.flush().await?;
        req.stream.close().await?;

        Ok(())
    })
}
