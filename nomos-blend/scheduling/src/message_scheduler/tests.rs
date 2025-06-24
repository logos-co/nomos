use core::{
    future::Future as _,
    num::NonZeroUsize,
    task::{Context, Poll},
    time::Duration,
};
use std::collections::HashSet;

use futures::{stream::empty, task::noop_waker_ref, StreamExt as _};
use rand::SeedableRng as _;
use rand_chacha::ChaCha20Rng;
use tokio::time::{interval_at, sleep, Instant};
use tokio_stream::{iter, wrappers::IntervalStream};

use crate::{
    cover_traffic_2::SessionCoverTraffic,
    message::OutboundMessage,
    message_scheduler::{
        round_info::{RoundClock, RoundInfo},
        session_info::{SendSessionClock, SessionInfo},
        MessageScheduler, Settings,
    },
    release_delayer::SessionProcessedMessageDelayer,
    UninitializedMessageScheduler,
};

async fn default_scheduler(
    session_duration: Duration,
    round_duration: Duration,
) -> MessageScheduler<SendSessionClock, ChaCha20Rng> {
    let rng = ChaCha20Rng::from_entropy();
    let session_clock = IntervalStream::new(interval_at(
        Instant::now() + session_duration,
        session_duration,
    ))
    .enumerate()
    .map(|(iteration, _)| SessionInfo {
        // First iteration is already considered started in the tests, so first clock
        // brings session `1` alive.
        session_number: u128::try_from(iteration + 1).unwrap().into(),
        core_quota: 1,
    });

    UninitializedMessageScheduler::new(
        Box::new(session_clock) as SendSessionClock,
        Settings {
            additional_safety_intervals: 0,
            expected_intervals_per_session: NonZeroUsize::new(2).unwrap(),
            maximum_release_delay_in_rounds: NonZeroUsize::new(1).unwrap(),
            round_duration,
            rounds_per_interval: NonZeroUsize::new(2).unwrap(),
        },
        rng,
    )
    .wait_next_session_start()
    .await
}

#[tokio::test]
async fn no_substream_ready() {
    let mut scheduler = default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
    let mut cx = Context::from_waker(noop_waker_ref());

    // We poll for round 0, which returns `Pending`, as per the default scheduler
    // configuration.
    sleep(Duration::from_millis(100)).await;
    assert!(scheduler.poll_next_unpin(&mut cx).is_pending());
    // We sleep for a bit more than one round.
    sleep(Duration::from_millis(600)).await; // We poll for round 1, which returns `Pending`.
    assert!(scheduler.poll_next_unpin(&mut cx).is_pending());
}

#[tokio::test]
async fn cover_traffic_substream_ready() {
    // Round 0 contains a cover message.
    let mut scheduler = {
        let mut scheduler =
            default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
        scheduler.cover_traffic = SessionCoverTraffic::with_test_values(
            Box::new(iter([0u128]).map(Into::into)) as RoundClock,
            HashSet::from([0u128.into()]),
            0,
        );

        scheduler
    };
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return a cover message as per override over
    // the default.
    sleep(Duration::from_millis(100)).await;
    let poll_result = scheduler.poll_next_unpin(&mut cx);
    assert_eq!(
        poll_result,
        Poll::Ready(Some(RoundInfo {
            cover_message: Some(vec![].into()),
            processed_messages: vec![]
        }))
    );
}

#[tokio::test]
async fn release_delayer_substream_ready() {
    // Round 0 contains processed messages.
    let mut scheduler = {
        let mut scheduler =
            default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
        scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
            NonZeroUsize::new(5).unwrap(),
            0u128.into(),
            ChaCha20Rng::from_entropy(),
            Box::new(iter([0u128]).map(Into::into)) as RoundClock,
            vec![OutboundMessage::from(b"test".to_vec())],
        );

        scheduler
    };
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return the processed messages as per override
    // over the default.
    sleep(Duration::from_millis(100)).await;
    let poll_result = scheduler.poll_next_unpin(&mut cx);
    assert_eq!(
        poll_result,
        Poll::Ready(Some(RoundInfo {
            cover_message: None,
            processed_messages: vec![OutboundMessage::from(b"test".to_vec())]
        }))
    );
}

#[tokio::test]
async fn both_substreams_ready() {
    // Round 0 contains both a cover message and processed messages.
    let mut scheduler = {
        let mut scheduler =
            default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
        scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
            NonZeroUsize::new(5).unwrap(),
            0u128.into(),
            ChaCha20Rng::from_entropy(),
            Box::new(iter([0u128]).map(Into::into)) as RoundClock,
            vec![OutboundMessage::from(b"test".to_vec())],
        );
        scheduler.cover_traffic = SessionCoverTraffic::with_test_values(
            Box::new(iter([0u128]).map(Into::into)) as RoundClock,
            HashSet::from([0u128.into()]),
            0,
        );

        scheduler
    };
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return the processed messages and a cover
    // message as per override over the default.
    sleep(Duration::from_millis(100)).await;
    let poll_result = scheduler.poll_next_unpin(&mut cx);
    assert_eq!(
        poll_result,
        Poll::Ready(Some(RoundInfo {
            cover_message: Some(vec![].into()),
            processed_messages: vec![OutboundMessage::from(b"test".to_vec())]
        }))
    );
}

#[tokio::test]
async fn round_change() {
    // Round 2 contains a cover message and round 2 contains a processed message.
    let mut scheduler = {
        let mut scheduler =
            default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
        scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
            NonZeroUsize::new(5).unwrap(),
            2u128.into(),
            ChaCha20Rng::from_entropy(),
            Box::new(iter([1u128, 2]).map(Into::into)) as RoundClock,
            vec![OutboundMessage::from(b"test".to_vec())],
        );
        scheduler.cover_traffic = SessionCoverTraffic::with_test_values(
            Box::new(iter([1u128, 2]).map(Into::into)) as RoundClock,
            HashSet::from([1u128.into()]),
            0,
        );

        scheduler
    };
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return `Pending`.
    sleep(Duration::from_millis(100)).await;
    let poll_result = scheduler.poll_next_unpin(&mut cx);
    assert_eq!(poll_result, Poll::Pending);

    // Poll for round 1, which should return a cover message.
    sleep(Duration::from_millis(500)).await;
    let poll_result = scheduler.poll_next_unpin(&mut cx);
    assert_eq!(
        poll_result,
        Poll::Ready(Some(RoundInfo {
            cover_message: Some(vec![].into()),
            processed_messages: vec![]
        }))
    );

    // Poll for round 2, which should return the processed messages.
    sleep(Duration::from_millis(500)).await;
    let poll_result = scheduler.poll_next_unpin(&mut cx);
    assert_eq!(
        poll_result,
        Poll::Ready(Some(RoundInfo {
            cover_message: None,
            processed_messages: vec![OutboundMessage::from(b"test".to_vec())]
        }))
    );
}

#[tokio::test]
async fn session_change() {
    let mut scheduler = {
        let mut scheduler =
            default_scheduler(Duration::from_secs(2), Duration::from_millis(500)).await;
        // We set the number of processed messages to `1`, so we can test the value is
        // reset on session changes.
        scheduler.cover_traffic = SessionCoverTraffic::with_test_values(
            Box::new(empty()) as RoundClock,
            HashSet::new(),
            1,
        );
        // We override the queue of unreleased messages, so we can test the value is
        // reset on session changes.
        scheduler.release_delayer = SessionProcessedMessageDelayer::with_test_values(
            NonZeroUsize::new(5).unwrap(),
            2u128.into(),
            ChaCha20Rng::from_entropy(),
            Box::new(empty()) as RoundClock,
            vec![OutboundMessage::from(b"test".to_vec())],
        );

        scheduler
    };
    assert_eq!(scheduler.cover_traffic.unprocessed_data_messages(), 1);
    assert_eq!(scheduler.release_delayer.unreleased_messages().len(), 1);
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll after new session. All the sub-streams should be reset.
    let mut waiting_time_fut = Box::pin(sleep(Duration::from_millis(3_500)));
    while waiting_time_fut.as_mut().poll(&mut cx).is_pending() {
        // We simulate polling the scheduler while waiting for the session to change.
        let _ = scheduler.poll_next_unpin(&mut cx);
        sleep(Duration::from_millis(500)).await;
    }

    assert_eq!(scheduler.cover_traffic.unprocessed_data_messages(), 0);
    assert_eq!(scheduler.release_delayer.unreleased_messages().len(), 0);
}
