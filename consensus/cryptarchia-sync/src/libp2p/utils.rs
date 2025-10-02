use futures::AsyncWriteExt as _;
use libp2p::PeerId;
use serde::{Serialize, de::DeserializeOwned};

use crate::libp2p::{errors::ChainSyncError, packing::pack_to_writer, stream::Stream};

pub async fn send_message<M: Serialize + DeserializeOwned + Sync>(
    peer_id: PeerId,
    mut stream: &mut Stream,
    message: &M,
) -> Result<(), ChainSyncError> {
    pack_to_writer(message, &mut stream)
        .await
        .map_err(|e| ChainSyncError::from((peer_id, e)))?;

    stream
        .flush()
        .await
        .map_err(|e| ChainSyncError::from((peer_id, e)))?;

    Ok(())
}
