use libp2p::Stream;
use nomos_core::wire::packing::{pack_to_writer, unpack_from_reader};
use tokio::sync::mpsc;

use crate::{
    messages::{Block, DownloadBlocksRequest, DownloadBlocksResponse},
    ChainSyncError,
};

pub const BUFFER_SIZE: usize = 64;

pub struct ProvideBlocksTask;

impl ProvideBlocksTask {
    pub async fn process_download_request(
        mut stream: Stream,
    ) -> Result<(Stream, DownloadBlocksRequest), ChainSyncError> {
        let request = unpack_from_reader(&mut stream).await?;
        Ok((stream, request))
    }

    pub async fn provide_blocks(
        mut reply_rcv: mpsc::Receiver<Block>,
        mut stream: Stream,
    ) -> Result<(), ChainSyncError> {
        while let Some(block) = reply_rcv.recv().await {
            if let Err(e) = pack_to_writer(&DownloadBlocksResponse::Block(block), &mut stream).await
            {
                return Err(ChainSyncError::PackingError(e));
            }
        }

        if let Err(e) = pack_to_writer(&DownloadBlocksResponse::NoMoreBlocks, &mut stream).await {
            return Err(ChainSyncError::PackingError(e));
        }

        Ok(())
    }
}
