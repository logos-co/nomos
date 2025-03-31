use futures::{future::BoxFuture, AsyncWriteExt, StreamExt};
use libp2p::Stream;
use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;

use crate::{
    behaviour::{
        BehaviourSyncReply, BlockResponse, IncomingSyncRequest, RequestKind, SyncError, SyncRequest,
    },
    sync_utils,
};

pub fn read_request_from_stream(
    mut stream: Stream,
) -> BoxFuture<'static, Result<IncomingSyncRequest, SyncError>> {
    Box::pin(async move {
        let command: SyncRequest = sync_utils::receive_data(&mut stream).await?;

        let kind = match command {
            SyncRequest::Sync { direction, slot } => {
                info!(direction = ?direction, slot = slot, "Received sync request");
                RequestKind::Sync { direction, slot }
            }
            SyncRequest::RequestTip => {
                info!("Received tip request");
                RequestKind::Tip
            }
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
                BehaviourSyncReply::TipData(tip) => {
                    info!(tip = tip, "Sending tip");
                    sync_utils::send_data(&mut req.stream, &tip).await?;
                }
                BehaviourSyncReply::Block(data) => {
                    let BlockResponse::Block(block) = data;
                    sync_utils::send_data(&mut req.stream, &block).await?;
                }
            }
            req.stream.flush().await?;
        }
        info!("Finished sending responses");
        req.stream.close().await?;
        Ok(())
    })
}
