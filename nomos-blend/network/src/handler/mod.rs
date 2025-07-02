use std::io;

use futures::{AsyncReadExt as _, AsyncWriteExt as _};
use libp2p::{Stream, StreamProtocol};

pub mod core;
pub use core::CoreToCoreBlendConnectionHandler;
pub mod edge;
pub use edge::CoreToEdgeBlendConnectionHandler;

const PROTOCOL_NAME: StreamProtocol = StreamProtocol::new("/nomos/blend/0.1.0");

/// Write a message to the stream
async fn send_msg(mut stream: Stream, msg: Vec<u8>) -> io::Result<Stream> {
    let msg_len: u16 = msg.len().try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Message length is too big. Got {}, expected {}",
                msg.len(),
                size_of::<u16>()
            ),
        )
    })?;
    stream.write_all(msg_len.to_be_bytes().as_ref()).await?;
    stream.write_all(&msg).await?;
    stream.flush().await?;
    Ok(stream)
}

/// Read a message from the stream
async fn recv_msg(mut stream: Stream) -> io::Result<(Stream, Vec<u8>)> {
    let mut msg_len = [0; size_of::<u16>()];
    stream.read_exact(&mut msg_len).await?;
    let msg_len = u16::from_be_bytes(msg_len) as usize;

    let mut buf = vec![0; msg_len];
    stream.read_exact(&mut buf).await?;
    Ok((stream, buf))
}
