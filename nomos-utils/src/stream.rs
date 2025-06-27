use core::{
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use std::sync::Arc;

use futures::{
    stream::{AbortHandle, Abortable},
    Stream, StreamExt as _,
};
use tokio::{
    spawn,
    sync::{
        broadcast::{channel, Receiver, Sender},
        Notify,
    },
    time::sleep,
};
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};
use tracing::error;

pub struct MultiConsumerStreamConstructor<InputStream, const CHANNEL_CAPACITY: usize>
where
    InputStream: Stream,
{
    stream_item_sender: Sender<InputStream::Item>,
    abort_handle: AbortHandle,
    channel_task_spawn_barrier: Arc<Notify>,
    ready_barrier: Arc<Notify>,
}

impl<InputStream, const CHANNEL_CAPACITY: usize>
    MultiConsumerStreamConstructor<InputStream, CHANNEL_CAPACITY>
where
    InputStream: Stream<Item: Clone + Debug + Send> + Send + Unpin + 'static,
{
    pub fn new(mut stream: InputStream) -> Self {
        let (stream_item_sender, _) = channel(CHANNEL_CAPACITY);
        let stream_item_sender_clone = stream_item_sender.clone();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();

        let channel_task_spawn_barrier = Arc::new(Notify::new());
        let channel_task_spawn_barrier_clone = Arc::clone(&channel_task_spawn_barrier);

        let ready_barrier = Arc::new(Notify::new());
        let ready_barrier_clone = Arc::clone(&ready_barrier);

        spawn(Abortable::new(
            async move {
                // Notify the `wait_ready` method that we are inside the async task.
                channel_task_spawn_barrier_clone.notify_waiters();
                // Wait for the `wait_ready` method to notify us that it's ready to consume
                // stream messages.
                ready_barrier_clone.notified().await;
                while let Some(next) = stream.next().await {
                    let _ = stream_item_sender_clone.send(next).inspect_err(|e| {
                        error!("Failed to forward stream element to receivers. Error {e:?}");
                    });
                }
            },
            abort_registration,
        ));

        Self {
            stream_item_sender,
            abort_handle,
            channel_task_spawn_barrier,
            ready_barrier,
        }
    }
}

impl<InputStream, const CHANNEL_CAPACITY: usize>
    MultiConsumerStreamConstructor<InputStream, CHANNEL_CAPACITY>
where
    InputStream: Stream<Item: Clone + Send + 'static>,
{
    #[must_use]
    pub fn new_consumer(&self) -> MultiConsumerStreamConsumer<InputStream::Item> {
        self.stream_item_sender.subscribe().into()
    }
}

impl<InputStream, const CHANNEL_CAPACITY: usize> From<InputStream>
    for MultiConsumerStreamConstructor<InputStream, CHANNEL_CAPACITY>
where
    InputStream: Stream<Item: Clone + Debug + Send> + Send + Unpin + 'static,
{
    fn from(value: InputStream) -> Self {
        Self::new(value)
    }
}

impl<InputStream, const CHANNEL_CAPACITY: usize>
    MultiConsumerStreamConstructor<InputStream, CHANNEL_CAPACITY>
where
    InputStream: Stream,
{
    pub async fn start(self) -> MultiConsumerStream<InputStream> {
        self.channel_task_spawn_barrier.notified().await;
        self.ready_barrier.notify_one();
        // Introduce a very small artificial delay to allow the consumers to be set up
        // before sending the stream items to them.
        sleep(Duration::from_micros(0)).await;
        MultiConsumerStream {
            stream_item_sender: self.stream_item_sender,
            abort_handle: self.abort_handle,
        }
    }
}

pub struct MultiConsumerStream<InputStream>
where
    InputStream: Stream,
{
    stream_item_sender: Sender<InputStream::Item>,
    abort_handle: AbortHandle,
}

impl<InputStream> MultiConsumerStream<InputStream>
where
    InputStream: Stream<Item: Clone + Send + 'static>,
{
    #[must_use]
    pub fn new_running_consumer(&self) -> MultiConsumerStreamConsumer<InputStream::Item> {
        self.stream_item_sender.subscribe().into()
    }
}

impl<InputStream> Drop for MultiConsumerStream<InputStream>
where
    InputStream: Stream,
{
    fn drop(&mut self) {
        self.abort_handle.abort();
        while !self.abort_handle.is_aborted() {}
    }
}

pub struct MultiConsumerStreamConsumer<T> {
    receiver_channel: Receiver<T>,
    receiver_stream: BroadcastStream<T>,
}

impl<T> From<Receiver<T>> for MultiConsumerStreamConsumer<T>
where
    T: Clone + Send + 'static,
{
    fn from(value: Receiver<T>) -> Self {
        Self {
            receiver_channel: value.resubscribe(),
            receiver_stream: value.into(),
        }
    }
}

impl<T> Clone for MultiConsumerStreamConsumer<T>
where
    T: Clone + Send + 'static,
{
    fn clone(&self) -> Self {
        let receiver_channel_clone = self.receiver_channel.resubscribe();
        receiver_channel_clone.into()
    }
}

impl<T> Stream for MultiConsumerStreamConsumer<T>
where
    T: Clone + Send + 'static,
{
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.receiver_stream.poll_next_unpin(cx) {
            Poll::Pending => Poll::Pending,
            // In this case, since we're inside a loop, we will keep polling the channel until
            // it returns a valid value.
            Poll::Ready(Some(Err(BroadcastStreamRecvError::Lagged(behind)))) => {
                error!("Failed to poll underlying broadcast receiver because lagging behind by {behind} values. Re-polling the channel to get the oldest available element...");
                self.poll_next(cx)
            }
            Poll::Ready(None) => Poll::Ready(None),
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
        let broadcast_stream = MultiConsumerStreamConstructor::<_, 3>::from(iter([1u8, 2u8, 3u8]));
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
        let broadcast_stream = MultiConsumerStreamConstructor::<_, 1>::from(empty::<()>());
        let mut consumer_1 = broadcast_stream.new_consumer();
        let mut consumer_2 = broadcast_stream.new_consumer();
        let new = broadcast_stream.start().await;
        let mut cx = Context::from_waker(noop_waker_ref());

        drop(new);

        assert_eq!(consumer_1.poll_next_unpin(&mut cx), Poll::Ready(None));
        assert_eq!(consumer_2.poll_next_unpin(&mut cx), Poll::Ready(None));
    }

    #[tokio::test]
    async fn handle_channel_lagging() {
        // Channel has capacity of `2`.
        let broadcast_stream =
            MultiConsumerStreamConstructor::<_, 2>::from(iter([1u8, 10u8, 20u8, 30u8, 40u8]));
        let mut consumer = broadcast_stream.new_consumer();
        broadcast_stream.start().await;
        // Polling the stream will return the oldest available element, which is `30`.
        assert_eq!(consumer.next().await, Some(30));
        assert_eq!(consumer.next().await, Some(40));
    }

    #[tokio::test]
    async fn handle_channel_none() {
        let broadcast_stream = MultiConsumerStreamConstructor::<_, 1>::from(empty::<()>());
        let mut cx = Context::from_waker(noop_waker_ref());

        let mut consumer = broadcast_stream.new_consumer();
        broadcast_stream.start().await;
        assert_eq!(consumer.poll_next_unpin(&mut cx), Poll::Ready(None));
    }

    #[tokio::test]
    async fn handle_channel_empty() {
        let broadcast_stream = MultiConsumerStreamConstructor::<_, 1>::from(pending());
        let mut cx = Context::from_waker(noop_waker_ref());

        let mut consumer: MultiConsumerStreamConsumer<()> = broadcast_stream.new_consumer();
        broadcast_stream.start().await;
        assert_eq!(consumer.poll_next_unpin(&mut cx), Poll::Pending);
    }
}
