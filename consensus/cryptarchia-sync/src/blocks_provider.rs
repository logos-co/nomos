use futures::{AsyncWriteExt as _, StreamExt as _};
use libp2p::{PeerId, Stream as Libp2pStream};
use nomos_core::wire::packing::{pack_to_writer, unpack_from_reader};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    errors::{ChainSyncError, ChainSyncErrorKind},
    messages::{DownloadBlocksRequest, DownloadBlocksResponse, SerialisedBlock},
};

pub const BUFFER_SIZE: usize = 64;

pub const MAX_ADDITIONAL_BLOCKS: usize = 5;

pub struct ProvideBlocksTask;

impl ProvideBlocksTask {
    pub async fn process_download_request(
        peer_id: PeerId,
        mut stream: Libp2pStream,
    ) -> Result<(PeerId, Libp2pStream, DownloadBlocksRequest), ChainSyncError> {
        let request: DownloadBlocksRequest = unpack_from_reader(&mut stream)
            .await
            .map_err(|e| ChainSyncError::from((peer_id, e)))?;

        if request.known_blocks.additional_blocks.len() > MAX_ADDITIONAL_BLOCKS {
            return Err(ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::ProtocolViolation(
                    "Too many additional blocks in request".to_owned(),
                ),
            });
        }

        Ok((peer_id, stream, request))
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
                    pack_to_writer(&DownloadBlocksResponse::Block(block), stream)
                        .await
                        .map_err(|e| ChainSyncError::from((peer_id, e)))?;

                    stream
                        .flush()
                        .await
                        .map_err(|e| ChainSyncError::from((peer_id, e)))?;

                    Ok(stream)
                },
            )
            .await?;

        if let Err(e) = pack_to_writer(&DownloadBlocksResponse::NoMoreBlocks, &mut stream).await {
            return Err(ChainSyncError::from((peer_id, e)));
        }

        Ok(())
    }
}
