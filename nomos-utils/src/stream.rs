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
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
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
        stream::{empty, iter},
        task::noop_waker_ref,
        StreamExt as _,
    };
    use tokio::time::{interval_at, sleep, Instant};
    use tokio_stream::wrappers::IntervalStream;

    use crate::stream::MultiConsumerStream;

    #[tokio::test]
    async fn propagate_some() {
        let broadcast_stream = MultiConsumerStream::from(iter([1u8, 2u8, 3u8]));
        let mut consumer_1 = broadcast_stream.new_consumer();
        let mut consumer_2 = broadcast_stream.new_consumer();

        assert_eq!(consumer_1.next().await, Some(1u8));
        assert_eq!(consumer_1.next().await, Some(2u8));
        assert_eq!(consumer_1.next().await, Some(3u8));

        assert_eq!(consumer_2.next().await, Some(1u8));
        assert_eq!(consumer_2.next().await, Some(2u8));
        assert_eq!(consumer_2.next().await, Some(3u8));
    }

    #[tokio::test]
    async fn different_subscriber_timings() {
        let interval = Duration::from_millis(100);
        let input_stream = IntervalStream::new(interval_at(Instant::now() + interval, interval))
            .enumerate()
            .map(|(i, _)| i);
        let broadcast_stream = MultiConsumerStream::from(input_stream);
        let mut consumer_1 = broadcast_stream.new_consumer();

        assert_eq!(consumer_1.next().await, Some(0usize));
        assert_eq!(consumer_1.next().await, Some(1usize));

        let mut consumer_2 = broadcast_stream.new_consumer();

        assert_eq!(consumer_1.next().await, Some(2usize));
        assert_eq!(consumer_2.next().await, Some(2usize));
    }

    #[tokio::test]
    async fn handle_channel_closing() {
        let broadcast_stream = MultiConsumerStream::from(empty::<()>());
        let mut consumer_1 = broadcast_stream.new_consumer();
        let mut consumer_2 = broadcast_stream.new_consumer();
        let mut cx = Context::from_waker(noop_waker_ref());

        drop(broadcast_stream);

        // Wait for stream to be dropped and the sender channel to be closed.
        sleep(Duration::from_millis(500)).await;

        assert_eq!(consumer_1.poll_next_unpin(&mut cx), Poll::Ready(None));
        assert_eq!(consumer_2.poll_next_unpin(&mut cx), Poll::Ready(None));
    }

    #[tokio::test]
    async fn handle_channel_lagging() {
        // Channel has capacity of `2`.
        let broadcast_stream =
            MultiConsumerStream::<_, 2>::new(iter([1u8, 10u8, 20u8, 30u8, 40u8]));

        let mut consumer = broadcast_stream.new_consumer();
        // Polling the stream will return the oldest available element, which is `30`.
        assert_eq!(consumer.next().await, Some(30));
        assert_eq!(consumer.next().await, Some(40));
    }

    #[tokio::test]
    async fn handle_channel_empty() {
        let broadcast_stream = MultiConsumerStream::from(empty::<()>());
        let mut cx = Context::from_waker(noop_waker_ref());

        // Wait for channel to be initialized
        sleep(Duration::from_millis(500)).await;

        let mut consumer = broadcast_stream.new_consumer();
        assert_eq!(consumer.poll_next_unpin(&mut cx), Poll::Pending);
    }
}
