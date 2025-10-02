use std::{
    io::{IoSlice, IoSliceMut},
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncWrite, AsyncWriteExt as _, FutureExt as _, future::BoxFuture};
use libp2p::PeerId;
use tokio::sync::oneshot;
use tracing::{debug, error};

use crate::{ChainSyncError, libp2p::behaviour::SYNC_PROTOCOL};

/// A wrapper around [`libp2p::Stream`] that ensures the stream is closed
/// properly when dropped.
pub struct Stream {
    /// The underlying libp2p stream.
    /// This is [`Option`] to allow taking it out in the [`Drop`].
    stream: Option<libp2p::Stream>,
    /// A sender to notify the [`StreamCloser`] when stream is dropped.
    /// This is [`Option`] to allow taking it out in the [`Drop`].
    drop_sender: Option<oneshot::Sender<libp2p::Stream>>,
}

/// A future that closes the libp2p stream when dropped.
/// The stream cannot be closed directly in the [`Drop`]
/// because it is an async operation.
pub type StreamCloser = BoxFuture<'static, ()>;

impl Stream {
    /// Returns a wrapped stream and a future that closes the stream when
    /// dropped.
    pub fn new(stream: libp2p::Stream) -> (Self, StreamCloser) {
        let (drop_sender, drop_receiver) = oneshot::channel::<libp2p::Stream>();
        let stream_closer = async move {
            if let Ok(mut stream) = drop_receiver.await {
                debug!("Closing libp2p stream...");
                if let Err(e) = stream.close().await {
                    error!("Failed to close libp2p stream: {e:?}");
                }
            }
        }
        .boxed();

        (
            Self {
                stream: Some(stream),
                drop_sender: Some(drop_sender),
            },
            stream_closer,
        )
    }

    /// Opens a new libp2p stream with the given peer ID,
    /// and wraps it in a [`Stream`].
    pub async fn open(
        peer_id: PeerId,
        control: &mut libp2p_stream::Control,
    ) -> Result<(Self, StreamCloser), ChainSyncError> {
        let stream = control
            .open_stream(peer_id, SYNC_PROTOCOL)
            .await
            .map_err(|e| ChainSyncError::from((peer_id, e)))?;
        Ok(Self::new(stream))
    }
}

/// Sends the libp2p stream to the stream closer when dropped.
impl Drop for Stream {
    fn drop(&mut self) {
        debug!("Dropping libp2p stream...");
        let stream = self.stream.take().expect("stream must exist");
        if self
            .drop_sender
            .take()
            .expect("drop_sender must exist")
            .send(stream)
            .is_err()
        {
            error!("Failed to send a dropped stream to the stream closer");
        }
    }
}

/// Implements [`AsyncRead`] for the underlying libp2p stream.
impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(self.get_mut().stream.as_mut().expect("stream must exist")).poll_read(cx, buf)
    }

    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(self.get_mut().stream.as_mut().expect("stream must exist"))
            .poll_read_vectored(cx, bufs)
    }
}

/// Implements [`AsyncWrite`] for the underlying libp2p stream.
impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(self.get_mut().stream.as_mut().expect("stream must exist")).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(self.get_mut().stream.as_mut().expect("stream must exist"))
            .poll_write_vectored(cx, bufs)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(self.get_mut().stream.as_mut().expect("stream must exist")).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(self.get_mut().stream.as_mut().expect("stream must exist")).poll_close(cx)
    }
}
