use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{future::BoxFuture, Future, FutureExt as _};
use libp2p::Stream;
use nomos_core::wire::packing::{pack_to_writer, unpack_from_reader, PackingError};
use tokio::sync::mpsc;
use Step::{Done, ReceivingBlockFromChannel, SendingBlockToNetwork, SendingCompleteToNetwork};

use crate::{
    messages::{Block, DownloadBlocksRequest, DownloadBlocksResponse},
    ChainSyncError,
};

type MessageSendFuture = BoxFuture<'static, Result<Stream, PackingError>>;

const BUFFER_SIZE: usize = 64;

enum Step {
    /// Waiting for a block from a channel.
    ReceivingBlockFromChannel(Stream),
    /// Processing sending a block to the peer.
    SendingBlockToNetwork(MessageSendFuture),
    /// Processing sending a completion message to the peer.
    SendingCompleteToNetwork(MessageSendFuture),
    /// Mark the task as done.
    Done,
}

pub struct ProvideBlocksTask {
    reply_rcv: mpsc::Receiver<Block>,
    current_step: Step,
}

impl ProvideBlocksTask {
    pub fn new(stream: Stream) -> (Self, mpsc::Sender<Block>) {
        let (reply_sender, reply_rcv) = mpsc::channel(BUFFER_SIZE);
        let task = Self {
            reply_rcv,
            current_step: ReceivingBlockFromChannel(stream),
        };

        (task, reply_sender)
    }

    pub async fn process_download_request(
        mut stream: Stream,
    ) -> Result<(Stream, DownloadBlocksRequest), ChainSyncError> {
        let request = unpack_from_reader(&mut stream).await?;
        Ok((stream, request))
    }

    fn handle_receiving_block_from_channel(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        mut stream: Stream,
    ) -> Poll<Result<(), ChainSyncError>> {
        match Pin::new(&mut self.reply_rcv).poll_recv(cx) {
            Poll::Pending => {
                self.current_step = ReceivingBlockFromChannel(stream);
                Poll::Pending
            }

            Poll::Ready(Some(block)) => {
                let fut = async move {
                    pack_to_writer(&DownloadBlocksResponse::Block(block), &mut stream)
                        .await
                        .map(|()| stream)
                }
                .boxed();

                self.current_step = SendingBlockToNetwork(fut);
                cx.waker().wake_by_ref();

                Poll::Pending
            }

            Poll::Ready(None) => {
                let sending_completion_fut = async move {
                    pack_to_writer(&DownloadBlocksResponse::NoMoreBlocks, &mut stream)
                        .await
                        .map(|()| stream)
                }
                .boxed();

                self.current_step = SendingCompleteToNetwork(sending_completion_fut);
                cx.waker().wake_by_ref();

                Poll::Pending
            }
        }
    }

    fn handle_sending_block_to_network(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        mut fut: MessageSendFuture,
    ) -> Poll<Result<(), ChainSyncError>> {
        match fut.as_mut().poll(cx) {
            Poll::Pending => {
                self.current_step = SendingBlockToNetwork(fut);
                Poll::Pending
            }

            Poll::Ready(Ok(stream)) => {
                self.current_step = ReceivingBlockFromChannel(stream);
                cx.waker().wake_by_ref();

                Poll::Pending
            }

            Poll::Ready(Err(e)) => Poll::Ready(Err(Into::into(e))),
        }
    }

    fn handle_sending_complete_to_network(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        mut fut: MessageSendFuture,
    ) -> Poll<Result<(), ChainSyncError>> {
        match fut.as_mut().poll(cx) {
            Poll::Pending => {
                self.current_step = SendingCompleteToNetwork(fut);
                Poll::Pending
            }

            Poll::Ready(Ok(_)) => {
                self.current_step = Done;
                Poll::Ready(Ok(()))
            }

            Poll::Ready(Err(e)) => Poll::Ready(Err(Into::into(e))),
        }
    }
}

impl Future for ProvideBlocksTask {
    type Output = Result<(), ChainSyncError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match std::mem::replace(&mut self.current_step, Done) {
            ReceivingBlockFromChannel(stream) => {
                self.handle_receiving_block_from_channel(cx, stream)
            }

            SendingBlockToNetwork(fut) => self.handle_sending_block_to_network(cx, fut),

            SendingCompleteToNetwork(fut) => self.handle_sending_complete_to_network(cx, fut),

            Done => Poll::Ready(Ok(())),
        }
    }
}
