use core::{
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{
    stream::{AbortHandle, Abortable},
    Stream, StreamExt as _,
};
use tokio::{
    spawn,
    sync::broadcast::{channel, Receiver, Sender},
};
use tracing::{error, trace};

pub struct MultiConsumerStream<T, const CHANNEL_CAPACITY: usize> {
    stream_item_sender: Sender<T>,
    abort_handle: AbortHandle,
}

impl<T, const CHANNEL_CAPACITY: usize> MultiConsumerStream<T, CHANNEL_CAPACITY>
where
    T: Clone + Debug + Send + 'static,
{
    pub fn new<InputStream>(mut stream: InputStream) -> Self
    where
        InputStream: Stream<Item = T> + Send + Unpin + 'static,
    {
        let (stream_item_sender, _) = channel(CHANNEL_CAPACITY);
        let sender_clone = stream_item_sender.clone();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();

        spawn(Abortable::new(
            async move {
                while let Some(next) = stream.next().await {
                    let _ = sender_clone.send(next).inspect_err(|e| {
                        error!("Failed to forward stream element to receivers. Error {e:?}");
                    });
                }
            },
            abort_registration,
        ));

        Self {
            stream_item_sender,
            abort_handle,
        }
    }
}

impl<T, const CHANNEL_CAPACITY: usize> MultiConsumerStream<T, CHANNEL_CAPACITY> {
    // Will call `Drop`, which will abort the task.
    pub fn close(self) {}
}

const DEFAULT_CHANNEL_CAPACITY: usize = 16;

impl<InputStream, T> From<InputStream> for MultiConsumerStream<T, DEFAULT_CHANNEL_CAPACITY>
where
    InputStream: Stream<Item = T> + Send + Unpin + 'static,
    T: Clone + Debug + Send + 'static,
{
    fn from(value: InputStream) -> Self {
        Self::new(value)
    }
}

impl<T, const CHANNEL_CAPACITY: usize> MultiConsumerStream<T, CHANNEL_CAPACITY>
where
    T: Clone + Send + 'static,
{
    #[must_use]
    pub fn new_consumer(&self) -> BroadcastStream<T> {
        self.stream_item_sender.subscribe().into()
    }
}

impl<T, const CHANNEL_CAPACITY: usize> Drop for MultiConsumerStream<T, CHANNEL_CAPACITY> {
    fn drop(&mut self) {
        trace!("Dropping broadcast channel.");
        self.abort_handle.abort();
    }
}

pub struct BroadcastStream<T> {
    receiver: tokio_stream::wrappers::BroadcastStream<T>,
}

impl<T> From<Receiver<T>> for BroadcastStream<T>
where
    T: Clone + Send + 'static,
{
    fn from(value: Receiver<T>) -> Self {
        Self {
            receiver: value.into(),
        }
    }
}

impl<T> Stream for BroadcastStream<T>
where
    T: Clone + Send + 'static,
{
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.receiver.poll_next_unpin(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Err(e))) => {
                error!("Failed to poll underlying broadcast receiver with error {e:?}. Returning `Pending` and waiting for the next element.");
                Poll::Pending
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(Ok(value))) => Poll::Ready(Some(value)),
        }
    }
}
