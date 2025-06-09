use futures::AsyncWriteExt as _;
use libp2p::{PeerId, Stream};
use libp2p_stream::Control;
use nomos_core::wire::packing::pack_to_writer;
use serde::Serialize;

use crate::{behaviour::SYNC_PROTOCOL, ChainSyncError};

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
