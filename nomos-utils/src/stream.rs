use core::{
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    time::Duration,
};

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
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tracing::{error, trace};

pub struct MultiConsumerStream<InputStream, const CHANNEL_CAPACITY: usize>
where
    InputStream: Stream,
{
    stream_item_sender: Sender<InputStream::Item>,
    abort_handle: AbortHandle,
    setup_barrier: Arc<Notify>,
    second_barrier: Arc<Notify>,
}

impl<InputStream, const CHANNEL_CAPACITY: usize> MultiConsumerStream<InputStream, CHANNEL_CAPACITY>
where
    InputStream: Stream<Item: Clone + Debug + Send + 'static> + Send + Unpin + 'static,
{
    pub fn new(mut stream: InputStream) -> Self {
        let (stream_item_sender, _) = channel(CHANNEL_CAPACITY);
        let sender_clone = stream_item_sender.clone();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();

        let setup_barrier = Arc::new(Notify::new());
        let setup_barrier_clone = Arc::clone(&setup_barrier);

        let second_barrier = Arc::new(Notify::new());
        let second_barrier_clone = Arc::clone(&second_barrier);

        spawn(Abortable::new(
            async move {
                setup_barrier_clone.notify_one();
                println!("Notified outer.");
                second_barrier_clone.notified().await;
                println!("Got notified by outer.");
                while let Some(next) = stream.next().await {
                    println!("Adding to queue.");
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
            setup_barrier,
            second_barrier,
        }
    }

    #[must_use]
    pub fn new_consumer(&self) -> MultiConsumerStreamConsumer<InputStream::Item> {
        self.stream_item_sender.subscribe().into()
    }

    pub async fn wait_ready(self) -> RunningMultiConsumerStream<InputStream, CHANNEL_CAPACITY> {
        self.setup_barrier.notified().await;
        println!("Got notified by inner.");
        self.second_barrier.notify_one();
        println!("Notified inner.");
        sleep(Duration::from_micros(0)).await;
        RunningMultiConsumerStream(self)
    }
}

const DEFAULT_CHANNEL_CAPACITY: usize = 16;

impl<InputStream> From<InputStream> for MultiConsumerStream<InputStream, DEFAULT_CHANNEL_CAPACITY>
where
    InputStream: Stream<Item: Clone + Debug + Send + 'static> + Send + Unpin + 'static,
{
    fn from(value: InputStream) -> Self {
        Self::new(value)
    }
}

pub struct RunningMultiConsumerStream<InputStream, const CHANNEL_CAPACITY: usize>(
    MultiConsumerStream<InputStream, CHANNEL_CAPACITY>,
)
where
    InputStream: Stream;

impl<InputStream, const CHANNEL_CAPACITY: usize> Drop
    for RunningMultiConsumerStream<InputStream, CHANNEL_CAPACITY>
where
    InputStream: Stream,
{
    fn drop(&mut self) {
        trace!("Dropping broadcast channel.");
        self.0.abort_handle.abort();
        while !self.0.abort_handle.is_aborted() {}
    }
}

pub struct MultiConsumerStreamConsumer<T> {
    receiver: tokio_stream::wrappers::BroadcastStream<T>,
}

impl<T> From<Receiver<T>> for MultiConsumerStreamConsumer<T>
where
    T: Clone + Send + 'static,
{
    fn from(value: Receiver<T>) -> Self {
        Self {
            receiver: value.into(),
        }
    }
}

impl<T> Stream for MultiConsumerStreamConsumer<T>
where
    T: Clone + Send + 'static,
{
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.receiver.poll_next_unpin(cx) {
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
    use core::{
        task::{Context, Poll},
        time::Duration,
    };

    use futures::{
        stream::{empty, iter, pending},
        task::noop_waker_ref,
        StreamExt as _,
    };
    use tokio::time::{interval_at, sleep, Instant};
    use tokio_stream::wrappers::IntervalStream;

    use crate::stream::{MultiConsumerStream, MultiConsumerStreamConsumer};

    #[tokio::test]
    async fn propagate_some() {
        let broadcast_stream = MultiConsumerStream::from(iter([1u8, 2u8, 3u8]));
        let mut consumer_1 = broadcast_stream.new_consumer();
        let mut consumer_2 = broadcast_stream.new_consumer();
        broadcast_stream.wait_ready().await;

        assert_eq!(consumer_1.next().await, Some(1u8));
        assert_eq!(consumer_1.next().await, Some(2u8));
        assert_eq!(consumer_1.next().await, Some(3u8));

        assert_eq!(consumer_2.next().await, Some(1u8));
        assert_eq!(consumer_2.next().await, Some(2u8));
        assert_eq!(consumer_2.next().await, Some(3u8));
    }

    #[tokio::test]
    async fn handle_channel_closing() {
        let broadcast_stream = MultiConsumerStream::from(empty::<()>());
        let mut consumer_1 = broadcast_stream.new_consumer();
        let mut consumer_2 = broadcast_stream.new_consumer();
        let new = broadcast_stream.wait_ready().await;
        let mut cx = Context::from_waker(noop_waker_ref());

        drop(new);

        assert_eq!(consumer_1.poll_next_unpin(&mut cx), Poll::Ready(None));
        assert_eq!(consumer_2.poll_next_unpin(&mut cx), Poll::Ready(None));
    }

    #[tokio::test]
    async fn handle_channel_lagging() {
        // Channel has capacity of `2`.
        let broadcast_stream =
            MultiConsumerStream::<_, 2>::new(iter([1u8, 10u8, 20u8, 30u8, 40u8]));
        let mut consumer = broadcast_stream.new_consumer();
        broadcast_stream.wait_ready().await;
        // Polling the stream will return the oldest available element, which is `30`.
        assert_eq!(consumer.next().await, Some(30));
        assert_eq!(consumer.next().await, Some(40));
    }

    #[tokio::test]
    async fn handle_channel_none() {
        let broadcast_stream = MultiConsumerStream::from(empty::<()>());
        let mut cx = Context::from_waker(noop_waker_ref());

        let mut consumer = broadcast_stream.new_consumer();
        broadcast_stream.wait_ready().await;
        assert_eq!(consumer.poll_next_unpin(&mut cx), Poll::Ready(None));
    }

    #[tokio::test]
    async fn handle_channel_empty() {
        let broadcast_stream = MultiConsumerStream::from(pending());
        let mut cx = Context::from_waker(noop_waker_ref());

        let mut consumer: MultiConsumerStreamConsumer<()> = broadcast_stream.new_consumer();
        broadcast_stream.wait_ready().await;
        assert_eq!(consumer.poll_next_unpin(&mut cx), Poll::Pending);
    }
}
