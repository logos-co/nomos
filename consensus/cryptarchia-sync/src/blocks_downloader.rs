use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{future::BoxFuture, AsyncWriteExt as _, FutureExt as _};
use libp2p::{PeerId, Stream};
use libp2p_stream::{Control, OpenStreamError};
use nomos_core::wire::packing::{pack_to_writer, unpack_from_reader, PackingError};
use tokio::sync::mpsc;
use tracing::error;

use crate::{
    behaviour::{BlocksResponse, SYNC_PROTOCOL},
    blocks_downloader::Step::{
        ClosingStream, Done, OpenStream, ReceivingMessage, SendingDownloadRequest,
        WaitingStreamToOpen,
    },
    messages::{DownloadBlocksRequest, DownloadBlocksResponse},
    ChainSyncError,
};

type OpenStreamFuture = BoxFuture<'static, Result<Stream, OpenStreamError>>;

type SendingDownloadRequestFuture = BoxFuture<'static, Result<Stream, PackingError>>;

type WaitingMessageFuture =
    BoxFuture<'static, (Stream, Result<DownloadBlocksResponse, PackingError>)>;

type ClosingStreamFuture = BoxFuture<'static, ()>;

const DOWNLOAD_BLOCKS_LIMIT: usize = 1000;

enum Step {
    /// Initiate the stream opening.
    OpenStream(Control),
    /// Waiting for the stream to be opened.
    WaitingStreamToOpen(OpenStreamFuture),
    /// Sending the download request to the peer.
    SendingDownloadRequest(SendingDownloadRequestFuture),
    /// Waiting next block.
    ReceivingMessage(WaitingMessageFuture),
    /// Mark the task as done.
    ClosingStream(ClosingStreamFuture),
    /// Mark the task as done.
    Done,
}

pub struct DownloadBlocksTask {
    current_step: Step,
    peer: PeerId,
    request: DownloadBlocksRequest,
    reply_tx: mpsc::Sender<BlocksResponse>,
    received_blocks: usize,
}

impl DownloadBlocksTask {
    pub const fn new(
        peer: PeerId,
        control: Control,
        request: DownloadBlocksRequest,
        blocks_tx: mpsc::Sender<BlocksResponse>,
    ) -> Self {
        Self {
            current_step: OpenStream(control),
            peer,
            request,
            reply_tx: blocks_tx,
            received_blocks: 0,
        }
    }

    fn handle_prepare_to_open_stream(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        mut control: Control,
    ) -> Poll<Result<(), ChainSyncError>> {
        let peer = self.peer;
        let open_stream_fut = async move { control.open_stream(peer, SYNC_PROTOCOL).await }.boxed();

        self.current_step = WaitingStreamToOpen(open_stream_fut);
        cx.waker().wake_by_ref();

        Poll::Pending
    }

    fn handle_wait_stream_to_open(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        mut fut: OpenStreamFuture,
    ) -> Poll<Result<(), ChainSyncError>> {
        match fut.as_mut().poll(cx) {
            Poll::Pending => {
                self.current_step = WaitingStreamToOpen(fut);
                Poll::Pending
            }

            Poll::Ready(Ok(mut stream)) => {
                let request = self.request.clone();
                let write_request_fut =
                    async move { pack_to_writer(&request, &mut stream).await.map(|()| stream) }
                        .boxed();

                self.current_step = SendingDownloadRequest(write_request_fut);
                cx.waker().wake_by_ref();

                Poll::Pending
            }

            Poll::Ready(Err(e)) => {
                self.report_error(&e.to_string());

                Poll::Ready(Err(Into::into(e)))
            }
        }
    }

    fn handle_sending_download_request(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        mut fut: SendingDownloadRequestFuture,
    ) -> Poll<Result<(), ChainSyncError>> {
        match fut.as_mut().poll(cx) {
            Poll::Pending => {
                self.current_step = SendingDownloadRequest(fut);
                Poll::Pending
            }

            Poll::Ready(Ok(mut stream)) => {
                let download_single_block_fut = async move {
                    let result = unpack_from_reader::<DownloadBlocksResponse, _>(&mut stream).await;
                    (stream, result)
                }
                .boxed();

                self.current_step = ReceivingMessage(download_single_block_fut);
                cx.waker().wake_by_ref();

                Poll::Pending
            }

            Poll::Ready(Err(e)) => {
                self.report_error(&e.to_string());

                Poll::Ready(Err(Into::into(e)))
            }
        }
    }

    fn handle_receiving_message(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        mut fut: WaitingMessageFuture,
    ) -> Poll<Result<(), ChainSyncError>> {
        match fut.as_mut().poll(cx) {
            Poll::Pending => {
                self.current_step = ReceivingMessage(fut);
                Poll::Pending
            }

            Poll::Ready((mut stream, Ok(DownloadBlocksResponse::Block(block)))) => {
                if self.received_blocks >= DOWNLOAD_BLOCKS_LIMIT {
                    let err_msg = format!(
                        "Peer exceeded DOWNLOAD_BLOCKS_LIMIT of {DOWNLOAD_BLOCKS_LIMIT} blocks"
                    );

                    self.received_blocks += 1;

                    if let Err(e) = self
                        .reply_tx
                        .try_send(BlocksResponse::NetworkError(err_msg.clone()))
                    {
                        return Poll::Ready(Err(Into::into(e)));
                    }

                    return Poll::Ready(Err(ChainSyncError::ProtocolViolation(err_msg)));
                }

                if let Err(e) = self.reply_tx.try_send(BlocksResponse::Block(block)) {
                    return Poll::Ready(Err(Into::into(e)));
                }

                let download_single_msg_fut = async move {
                    let result = unpack_from_reader::<DownloadBlocksResponse, _>(&mut stream).await;
                    (stream, result)
                }
                .boxed();

                self.current_step = ReceivingMessage(download_single_msg_fut);
                cx.waker().wake_by_ref();

                Poll::Pending
            }

            Poll::Ready((mut stream, Ok(DownloadBlocksResponse::NoMoreBlocks))) => {
                self.current_step = ClosingStream(
                    async move {
                        let _ = stream.close().await;
                    }
                    .boxed(),
                );
                cx.waker().wake_by_ref();

                Poll::Ready(Ok(()))
            }

            Poll::Ready((_, Err(e))) => {
                self.report_error(&e.to_string());

                Poll::Ready(Err(Into::into(e)))
            }
        }
    }

    fn handle_closing_stream(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        mut fut: ClosingStreamFuture,
    ) -> Poll<Result<(), ChainSyncError>> {
        match fut.as_mut().poll(cx) {
            Poll::Pending => {
                self.current_step = ClosingStream(fut);
                Poll::Pending
            }

            Poll::Ready(()) => Poll::Ready(Ok(())),
        }
    }

    fn report_error(self: Pin<&mut Self>, error: &str) -> Option<Poll<Result<(), ChainSyncError>>> {
        if let Err(e) = self
            .reply_tx
            .try_send(BlocksResponse::NetworkError(error.to_owned()))
        {
            error!("Error sending error response: {:?}", e);
        }
        None
    }
}

impl Future for DownloadBlocksTask {
    type Output = Result<(), ChainSyncError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match std::mem::replace(&mut self.current_step, Done) {
            OpenStream(control) => self.handle_prepare_to_open_stream(cx, control),

            WaitingStreamToOpen(fut) => self.handle_wait_stream_to_open(cx, fut),

            SendingDownloadRequest(fut) => self.handle_sending_download_request(cx, fut),

            ReceivingMessage(fut) => self.handle_receiving_message(cx, fut),

            ClosingStream(fut) => self.handle_closing_stream(cx, fut),

            Done => Poll::Ready(Ok(())),
        }
    }
}
