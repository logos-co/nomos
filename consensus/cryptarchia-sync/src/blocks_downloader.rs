use futures::stream::BoxStream;
use libp2p::{PeerId, Stream as Libp2pStream, Stream};
use libp2p_stream::Control;
use nomos_core::{header::HeaderId, wire::packing::unpack_from_reader};

use crate::{
    behaviour::BlocksResponse,
    errors::ChainSyncError,
    messages::{
        DownloadBlocksRequest, DownloadBlocksResponse, GetTipRequest, GetTipResponse,
        RequestMessage,
    },
    util::{open_stream, send_message},
    DownloadBlocksInfo,
};

pub struct DownloadBlocksTask;

impl DownloadBlocksTask {
    pub async fn send_request(
        peer_id: PeerId,
        mut control: Control,
        request: DownloadBlocksInfo,
    ) -> Result<(PeerId, Libp2pStream), ChainSyncError> {
        let tip_response = Self::send_tip_request(peer_id, &mut control).await?;

        let result: (PeerId, Libp2pStream) =
            Self::send_download_request(peer_id, &mut control, request, tip_response.tip).await?;

        Ok(result)
    }

    async fn send_tip_request(
        peer_id: PeerId,
        control: &mut Control,
    ) -> Result<GetTipResponse, ChainSyncError> {
        let mut stream = open_stream(peer_id, control).await?;

        let tip_request = RequestMessage::GetTipRequest(GetTipRequest);
        send_message(peer_id, &mut stream, &tip_request).await?;

        let response: GetTipResponse = unpack_from_reader(&mut stream)
            .await
            .map_err(|e| ChainSyncError::from((peer_id, e)))?;

        Ok(response)
    }

    async fn send_download_request(
        peer_id: PeerId,
        control: &mut Control,
        request: DownloadBlocksInfo,
        peer_tip: HeaderId,
    ) -> Result<(PeerId, Stream), ChainSyncError> {
        let mut stream = open_stream(peer_id, control).await?;

        let target_block = request.target_block.unwrap_or(peer_tip);
        let request = DownloadBlocksRequest::new(
            target_block,
            request.local_tip,
            request.latest_immutable_block,
            request.additional_blocks,
        );

        let download_request = RequestMessage::DownloadBlocksRequest(request);
        send_message(peer_id, &mut stream, &download_request).await?;

        Ok((peer_id, stream))
    }

    pub fn download_blocks(
        peer_id: PeerId,
        stream: Libp2pStream,
    ) -> BoxStream<'static, Result<BlocksResponse, ChainSyncError>> {
        Box::pin(futures::stream::try_unfold(
            stream,
            move |mut stream| async move {
                match unpack_from_reader::<DownloadBlocksResponse, _>(&mut stream).await {
                    Ok(DownloadBlocksResponse::Block(block)) => {
                        Ok(Some((BlocksResponse::Block((peer_id, block)), stream)))
                    }
                    Ok(DownloadBlocksResponse::NoMoreBlocks) => Ok(None),
                    Err(e) => Err(ChainSyncError::from((peer_id, e))),
                }
            },
        ))
    }
}
