use futures::AsyncWriteExt as _;
use libp2p::{PeerId, Stream};
use libp2p_stream::Control;
use serde::Serialize;

use crate::libp2p::{behaviour::SYNC_PROTOCOL, errors::ChainSyncError, packing::pack_to_writer};

pub async fn send_message<M: Serialize + Sync>(
    peer_id: PeerId,
    mut stream: &mut Stream,
    message: &M,
) -> Result<(), ChainSyncError> {
    pack_to_writer(&message, &mut stream)
        .await
        .map_err(|e| ChainSyncError::from((peer_id, e)))?;

    stream
        .flush()
        .await
        .map_err(|e| ChainSyncError::from((peer_id, e)))?;

    Ok(())
}

pub async fn open_stream(peer_id: PeerId, control: &mut Control) -> Result<Stream, ChainSyncError> {
    let stream = control
        .open_stream(peer_id, SYNC_PROTOCOL)
        .await
        .map_err(|e| ChainSyncError::from((peer_id, e)))?;
    Ok(stream)
}

pub async fn close_stream(peer_id: PeerId, mut stream: Stream) -> Result<(), ChainSyncError> {
    stream
        .flush()
        .await
        .map_err(|e| ChainSyncError::from((peer_id, e)))?;

    stream
        .close()
        .await
        .map_err(|e| ChainSyncError::from((peer_id, e)))?;
    Ok(())
}
