use futures::{future::BoxFuture, AsyncWriteExt, StreamExt};
use libp2p::Stream;
use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    behaviour::{BehaviourSyncReply, IncomingSyncRequest, RequestKind, SyncError, SyncRequest},
    messages::SyncPeerMessage,
    sync_utils,
};

pub fn read_request_from_stream(
    mut stream: Stream,
) -> BoxFuture<'static, Result<IncomingSyncRequest, SyncError>> {
    Box::pin(async move {
        let command: SyncRequest = sync_utils::receive_data(&mut stream).await?;

        let kind = match command {
            SyncRequest::Sync { direction } => RequestKind::Sync { direction },
            SyncRequest::RequestTip => RequestKind::Tip,
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

pub fn send_response_to_peer(
    mut req: IncomingSyncRequest,
) -> BoxFuture<'static, Result<(), SyncError>> {
    Box::pin(async move {
        let mut response_receiver = req.response_stream;

        while let Some(service_response) = response_receiver.next().await {
            match service_response {
                BehaviourSyncReply::Block(block) => {
                    let message = SyncPeerMessage::Block(block);
                    sync_utils::send_data(&mut req.stream, &message).await?;
                }
                BehaviourSyncReply::TipData(tip) => {
                    let message = SyncPeerMessage::TipData(tip);
                    sync_utils::send_data(&mut req.stream, &message).await?;
                }
            }
            req.stream.flush().await?;
        }

        sync_utils::send_data(&mut req.stream, &SyncPeerMessage::End).await?;

        req.stream.flush().await?;
        req.stream.close().await?;

        Ok(())
    })
}
