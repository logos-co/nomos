use futures::AsyncWriteExt as _;
use libp2p::PeerId;
use libp2p_stream::Control;
use nomos_core::wire::packing::{pack_to_writer, unpack_from_reader};
use tokio::sync::mpsc;

use crate::{
    behaviour::{BlocksResponse, SYNC_PROTOCOL},
    messages::{DownloadBlocksRequest, DownloadBlocksResponse},
    ChainSyncError,
};

const DOWNLOAD_BLOCKS_LIMIT: usize = 1000;

pub struct DownloadBlocksTask {
    request: DownloadBlocksRequest,
    reply_tx: mpsc::Sender<BlocksResponse>,
    received_blocks: usize,
}

impl DownloadBlocksTask {
    pub const fn new(
        request: DownloadBlocksRequest,
        reply_tx: mpsc::Sender<BlocksResponse>,
    ) -> Self {
        Self {
            request,
            reply_tx,
            received_blocks: 0,
        }
    }

    pub async fn download_blocks(
        &mut self,
        peer_id: PeerId,
        mut control: Control,
    ) -> Result<(), ChainSyncError> {
        let mut stream = match control.open_stream(peer_id, SYNC_PROTOCOL).await {
            Ok(s) => s,
            Err(e) => {
                self.reply_tx
                    .send(BlocksResponse::NetworkError(e.to_string()))
                    .await?;
                return Err(e.into());
            }
        };

        if let Err(e) = pack_to_writer(&self.request, &mut stream).await {
            self.reply_tx
                .send(BlocksResponse::NetworkError(e.to_string()))
                .await?;
            return Err(Into::into(e));
        }

        loop {
            match unpack_from_reader::<DownloadBlocksResponse, _>(&mut stream).await {
                Ok(response) => match response {
                    DownloadBlocksResponse::Block(block) => {
                        if self.received_blocks >= DOWNLOAD_BLOCKS_LIMIT {
                            let err_msg = format!(
                                "Peer exceeded DOWNLOAD_BLOCKS_LIMIT of {DOWNLOAD_BLOCKS_LIMIT} blocks"
                            );
                            self.reply_tx
                                .send(BlocksResponse::NetworkError(err_msg.clone()))
                                .await?;

                            return Err(ChainSyncError::ProtocolViolation(err_msg));
                        }
                        self.reply_tx.send(BlocksResponse::Block(block)).await?;
                        self.received_blocks += 1;
                    }
                    DownloadBlocksResponse::NoMoreBlocks => break,
                },
                Err(e) => {
                    self.reply_tx
                        .send(BlocksResponse::NetworkError(e.to_string()))
                        .await?;
                    return Err(Into::into(e));
                }
            }
        }

        stream.close().await.map_err(Into::into)
    }
}
