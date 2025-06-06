use futures::AsyncWriteExt as _;
use libp2p::Stream as Libp2pStream;
use nomos_core::wire::packing::{pack_to_writer, unpack_from_reader};
use tokio::sync::mpsc;

use crate::{
    messages::{DownloadBlocksRequest, DownloadBlocksResponse, SerialisedBlock},
    ChainSyncError,
};

pub const BUFFER_SIZE: usize = 64;

pub struct ProvideBlocksTask;

impl ProvideBlocksTask {
    pub async fn process_download_request(
        mut stream: Libp2pStream,
    ) -> Result<(Libp2pStream, DownloadBlocksRequest), ChainSyncError> {
        let request = unpack_from_reader(&mut stream).await?;
        Ok((stream, request))
    }

    pub async fn provide_blocks(
        mut reply_rcv: mpsc::Receiver<SerialisedBlock>,
        mut stream: Libp2pStream,
    ) -> Result<(), ChainSyncError> {
        while let Some(block) = reply_rcv.recv().await {
            if let Err(e) = pack_to_writer(&DownloadBlocksResponse::Block(block), &mut stream).await
            {
                return Err(ChainSyncError::PackingError(e));
            }
            stream.flush().await?;
        }

        if let Err(e) = pack_to_writer(&DownloadBlocksResponse::NoMoreBlocks, &mut stream).await {
            return Err(ChainSyncError::PackingError(e));
        }

        Ok(())
    }
}
