use futures::stream::BoxStream;
use libp2p::{PeerId, Stream as Libp2pStream};
use libp2p_stream::Control;
use nomos_core::wire::packing::{pack_to_writer, unpack_from_reader};

use crate::{
    behaviour::{BlocksResponse, ChainSyncErrorKind, SYNC_PROTOCOL},
    messages::{DownloadBlocksRequest, DownloadBlocksResponse},
    ChainSyncError,
};

const DOWNLOAD_BLOCKS_LIMIT: usize = 1000;

pub struct DownloadBlocksTask;

impl DownloadBlocksTask {
    pub async fn send_request(
        peer_id: PeerId,
        mut control: Control,
        request: DownloadBlocksRequest,
    ) -> Result<(PeerId, Libp2pStream), ChainSyncError> {
        let mut stream = match control.open_stream(peer_id, SYNC_PROTOCOL).await {
            Ok(s) => s,
            Err(e) => {
                return Err(ChainSyncError {
                    peer: peer_id,
                    kind: ChainSyncErrorKind::OpenStreamError(e),
                });
            }
        };

        if let Err(e) = pack_to_writer(&request, &mut stream).await {
            return Err(ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::PackingError(e),
            });
        }

        Ok((peer_id, stream))
    }
    pub fn download_blocks(
        peer_id: PeerId,
        stream: Libp2pStream,
    ) -> BoxStream<'static, Result<BlocksResponse, ChainSyncError>> {
        let received_blocks = 0usize;
        Box::pin(futures::stream::try_unfold(
            (stream, received_blocks),
            move |(mut stream, count)| async move {
                match unpack_from_reader::<DownloadBlocksResponse, _>(&mut stream).await {
                    Ok(DownloadBlocksResponse::Block(block)) => {
                        let Some(count) = count.checked_add(1) else {
                            return Err(ChainSyncError {
                                peer: peer_id,
                                kind: ChainSyncErrorKind::ProtocolViolation(
                                    "Block count overflow".to_owned(),
                                ),
                            });
                        };

                        if count >= DOWNLOAD_BLOCKS_LIMIT {
                            let msg = format!("Peer exceeded DOWNLOAD_BLOCKS_LIMIT of {DOWNLOAD_BLOCKS_LIMIT} blocks");
                            return Err(ChainSyncError {
                                peer: peer_id,
                                kind: ChainSyncErrorKind::ProtocolViolation(msg),
                            });
                        }
                        Ok(Some((
                            BlocksResponse::Block((peer_id, block)),
                            (stream, count + 1),
                        )))
                    }
                    Ok(DownloadBlocksResponse::NoMoreBlocks) => Ok(None),
                    Err(e) => {
                        let msg = format!("Failed to unpack DownloadBlocksResponse: {e}");
                        Err(ChainSyncError {
                            peer: peer_id,
                            kind: ChainSyncErrorKind::ProtocolViolation(msg),
                        })
                    }
                }
            },
        ))
    }
}
