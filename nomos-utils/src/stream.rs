use core::{
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
};
use std::sync::Arc;

use futures::{
    stream::{AbortHandle, Abortable},
    Stream, StreamExt as _,
};
use tokio::{
    spawn,
    sync::{
        broadcast::{channel, Sender},
        Notify,
    },
};
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};
use tracing::error;

/// A wrapper around a stream that allows to mak it multi-consumer.
///
/// It allows to create one or multiple consumers before it is started.
pub struct MultiConsumerStreamConstructor<InputStream>
where
    InputStream: Stream,
{
    /// The broadcast sender used to create receivers upon new consumer
    /// requests.
    stream_item_sender: Sender<InputStream::Item>,
    /// The input stream to forward to consumers.
    input_stream: InputStream,
}

impl<InputStream> MultiConsumerStreamConstructor<InputStream>
where
    InputStream: Stream<Item: Clone + Debug + Send> + Send + Unpin + 'static,
{
    /// Initialize the constructor with the provided stream and channel
    /// capacity.
    ///
    /// The channel capacity should be tailored to the nature of the stream:
    /// high-frequency streams might require a higher throughput channel. In
    /// case of more pending items than the allowed space, older values are
    /// replaced with the newest ones, and sub-streams that have not polled the
    /// discarded values will receive the oldest value in memory at the time
    /// they poll the stream.
    pub fn new(input_stream: InputStream, channel_capacity: usize) -> Self {
        let (stream_item_sender, _) = channel(channel_capacity);

        Self {
            stream_item_sender,
            input_stream,
        }
    }
}

impl<InputStream> MultiConsumerStreamConstructor<InputStream>
where
    InputStream: Stream<Item: Clone + Send + 'static>,
{
    /// Create a new consumer for the stream.
    #[must_use]
    pub fn new_consumer(&self) -> MultiConsumerStreamConsumer<InputStream::Item> {
        MultiConsumerStreamConsumer {
            receiver_stream: self.stream_item_sender.subscribe().into(),
        }
    }
}

pub const DEFAULT_CHANNEL_CAPACITY: usize = 16;

// Create a multi-consumer stream constructor from the provided stream, using a
// default channel capacity of [`DEFAULT_CHANNEL_CAPACITY`].
impl<InputStream> From<InputStream> for MultiConsumerStreamConstructor<InputStream>
where
    InputStream: Stream<Item: Clone + Debug + Send> + Send + Unpin + 'static,
{
    fn from(value: InputStream) -> Self {
        Self::new(value, DEFAULT_CHANNEL_CAPACITY)
    }
}

impl<InputStream> MultiConsumerStreamConstructor<InputStream>
where
    InputStream: Stream<Item: Debug + Send> + Send + Unpin + 'static,
{
    /// Start the multi-consumer stream, consuming the constructor and returning
    /// the stream itself.
    ///
    /// All created consumers will start receiving the stream values.
    pub async fn start(self) -> MultiConsumerStream {
        let Self {
            mut input_stream,
            stream_item_sender,
        } = self;

        let channel_task_spawn_barrier = Arc::new(Notify::new());
        let channel_task_spawn_barrier_clone = Arc::clone(&channel_task_spawn_barrier);

        let stream_item_sender_clone = stream_item_sender.clone();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();

        spawn(Abortable::new(
            async move {
                // Notify the outer task that we are ready to produce values.
                channel_task_spawn_barrier_clone.notify_waiters();
                while let Some(next) = input_stream.next().await {
                    let _ = stream_item_sender_clone.send(next).inspect_err(|e| {
                        error!("Failed to forward stream element to receivers. Error {e:?}");
                    });
                }
            },
            abort_registration,
        ));
        // Wait until the inner stream is ready before returning to the caller.
        channel_task_spawn_barrier.notified().await;

        MultiConsumerStream { abort_handle }
    }
}

/// A running multi-consumer stream.
pub struct MultiConsumerStream {
    abort_handle: AbortHandle,
}

impl Drop for MultiConsumerStream {
    fn drop(&mut self) {
        self.abort_handle.abort();
        while !self.abort_handle.is_aborted() {}
    }
}

/// A consumer instance of a multi-consumer stream.
pub struct MultiConsumerStreamConsumer<T> {
    /// The stream created from the wrapped broadcast receiver. This is used in
    /// the `Stream` implementation for this type.
    receiver_stream: BroadcastStream<T>,
}

impl<T> Stream for MultiConsumerStreamConsumer<T>
where
    T: Clone + Send + 'static,
{
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.receiver_stream.poll_next_unpin(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            // In case the receiver has lagged behind, we poll the receiver once again to retrieve
            // the oldest value available in the channel.
            Poll::Ready(Some(Err(BroadcastStreamRecvError::Lagged(behind)))) => {
                error!("Failed to poll underlying broadcast receiver because lagging behind by {behind} values. Re-polling the channel to get the oldest available element...");
                self.poll_next(cx)
            }
            Poll::Ready(Some(Ok(value))) => Poll::Ready(Some(value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::task::{Context, Poll};

    use futures::{
        stream::{empty, iter, pending},
        task::noop_waker_ref,
        StreamExt as _,
    };

    use crate::stream::{MultiConsumerStreamConstructor, MultiConsumerStreamConsumer};

    #[tokio::test]
    async fn propagate_some() {
        let broadcast_stream = MultiConsumerStreamConstructor::from(iter([1u8, 2u8, 3u8]));
        let mut consumer_1 = broadcast_stream.new_consumer();
        let mut consumer_2 = broadcast_stream.new_consumer();
        broadcast_stream.start().await;

        assert_eq!(consumer_1.next().await, Some(1u8));
        assert_eq!(consumer_1.next().await, Some(2u8));
        assert_eq!(consumer_1.next().await, Some(3u8));

        assert_eq!(consumer_2.next().await, Some(1u8));
        assert_eq!(consumer_2.next().await, Some(2u8));
        assert_eq!(consumer_2.next().await, Some(3u8));
    }

    #[tokio::test]
    async fn handle_channel_closing() {
        let broadcast_stream = MultiConsumerStreamConstructor::from(empty::<()>());
        let mut consumer_1 = broadcast_stream.new_consumer();
        let mut consumer_2 = broadcast_stream.new_consumer();
        let running_stream = broadcast_stream.start().await;
        let mut cx = Context::from_waker(noop_waker_ref());

        drop(running_stream);

        assert_eq!(consumer_1.poll_next_unpin(&mut cx), Poll::Ready(None));
        assert_eq!(consumer_2.poll_next_unpin(&mut cx), Poll::Ready(None));
    }

    #[tokio::test]
    async fn handle_channel_lagging() {
        // Channel has capacity of `2`.
        let broadcast_stream =
            MultiConsumerStreamConstructor::new(iter([1u8, 10u8, 20u8, 30u8, 40u8]), 2);
        let mut consumer = broadcast_stream.new_consumer();
        broadcast_stream.start().await;
        // Polling the stream will return the oldest available element, which is `30`.
        assert_eq!(consumer.next().await, Some(30));
        assert_eq!(consumer.next().await, Some(40));
    }

    #[tokio::test]
    async fn handle_channel_none() {
        let broadcast_stream = MultiConsumerStreamConstructor::from(empty::<()>());
        let mut cx = Context::from_waker(noop_waker_ref());

        let mut consumer = broadcast_stream.new_consumer();
        broadcast_stream.start().await;
        assert_eq!(consumer.poll_next_unpin(&mut cx), Poll::Ready(None));
    }

    #[tokio::test]
    async fn handle_channel_empty() {
        let broadcast_stream = MultiConsumerStreamConstructor::from(pending());
        let mut cx = Context::from_waker(noop_waker_ref());

        let mut consumer: MultiConsumerStreamConsumer<()> = broadcast_stream.new_consumer();
        broadcast_stream.start().await;
        assert_eq!(consumer.poll_next_unpin(&mut cx), Poll::Pending);
    }
}
