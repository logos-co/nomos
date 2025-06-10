use futures::StreamExt as _;
use libp2p::{PeerId, Stream as Libp2pStream};
use nomos_core::{header::HeaderId, wire::packing::unpack_from_reader};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    errors::{ChainSyncError, ChainSyncErrorKind},
    messages::{DownloadBlocksResponse, GetTipResponse, RequestMessage, SerialisedBlock},
    util::send_message,
};

pub const BUFFER_SIZE: usize = 64;

pub const MAX_ADDITIONAL_BLOCKS: usize = 5;

pub struct ProvideBlocksTask;

impl ProvideBlocksTask {
    pub async fn process_request(
        peer_id: PeerId,
        mut stream: Libp2pStream,
    ) -> Result<(PeerId, Libp2pStream, RequestMessage), ChainSyncError> {
        let request: RequestMessage = unpack_from_reader(&mut stream)
            .await
            .map_err(|e| ChainSyncError::from((peer_id, e)))?;

        Ok((peer_id, stream, request))
    }

    pub async fn provide_tip(
        mut reply_rcv: mpsc::Receiver<HeaderId>,
        peer_id: PeerId,
        mut stream: Libp2pStream,
    ) -> Result<(), ChainSyncError> {
        match reply_rcv.recv().await {
            Some(tip) => {
                let response = GetTipResponse { tip };
                send_message(peer_id, &mut stream, &response).await?;

                Ok(())
            }
            None => Err(ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::ChannelReceiveError(
                    "No tip received in response to GetTipRequest".to_owned(),
                ),
            }),
        }
    }

    pub async fn provide_blocks(
        reply_rcv: mpsc::Receiver<SerialisedBlock>,
        peer_id: PeerId,
        mut stream: Libp2pStream,
    ) -> Result<(), ChainSyncError> {
        ReceiverStream::new(reply_rcv)
            .fold(
                Ok(&mut stream),
                |res: Result<&mut _, ChainSyncError>, block| async move {
                    let stream = res?;
                    let message = DownloadBlocksResponse::Block(block);
                    send_message(peer_id, stream, &message).await?;
                    Ok(stream)
                },
            )
            .await?;

        let request = DownloadBlocksResponse::NoMoreBlocks;
        send_message(peer_id, &mut stream, &request).await?;

        Ok(())
    }
}
