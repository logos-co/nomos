use std::io::Error as IoError;

use futures::AsyncWriteExt;
use libp2p::{PeerId, Stream, StreamProtocol};
use libp2p_stream::{Control, OpenStreamError};
use nomos_core::wire::packing::{pack_to_writer, unpack_from_reader};
use serde::{Serialize, de::DeserializeOwned};

pub async fn send_data<T: Serialize + Sync>(stream: &mut Stream, data: &T) -> Result<(), IoError> {
    pack_to_writer(data, stream).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn receive_data<T: DeserializeOwned>(stream: &mut Stream) -> Result<T, IoError> {
    unpack_from_reader(stream).await
}

#[inline]
pub async fn open_stream(
    peer_id: PeerId,
    control: &mut Control,
    protocol: StreamProtocol,
) -> Result<Stream, OpenStreamError> {
    control.open_stream(peer_id, protocol).await
}
