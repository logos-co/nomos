use futures::stream::BoxStream;
use libp2p::{PeerId, Stream};
use libp2p_stream::Control;
use nomos_core::wire::packing::{pack_to_writer, unpack_from_reader};

use crate::{
    behaviour::{BlocksResponse, SYNC_PROTOCOL},
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
    ) -> Result<Stream, ChainSyncError> {
        let mut stream = match control.open_stream(peer_id, SYNC_PROTOCOL).await {
            Ok(s) => s,
            Err(e) => {
                return Err(e.into());
            }
        };

        if let Err(e) = pack_to_writer(&request, &mut stream).await {
            return Err(Into::into(e));
        }

        Ok(stream)
    }
    pub fn download_blocks(
        stream: Stream,
    ) -> BoxStream<'static, Result<BlocksResponse, ChainSyncError>> {
        let received_blocks = 0usize;
        Box::pin(futures::stream::try_unfold(
            (stream, received_blocks),
            |(mut stream, count)| async move {
                match unpack_from_reader::<DownloadBlocksResponse, _>(&mut stream).await {
                    Ok(DownloadBlocksResponse::Block(block)) => {
                        let Some(count) = count.checked_add(1) else {
                            return Err(ChainSyncError::ProtocolViolation(
                                "Block count overflowed".to_owned(),
                            ));
                        };

                        if count >= DOWNLOAD_BLOCKS_LIMIT {
                            let msg = format!("Peer exceeded DOWNLOAD_BLOCKS_LIMIT of {DOWNLOAD_BLOCKS_LIMIT} blocks");
                            return Err(ChainSyncError::ProtocolViolation(msg));
                        }
                        Ok(Some((BlocksResponse::Block(block), (stream, count + 1))))
                    }
                    Ok(DownloadBlocksResponse::NoMoreBlocks) => Ok(None),
                    Err(e) => Err(ChainSyncError::PackingError(e)),
                }
            },
        ))
    }
}
