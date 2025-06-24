use core::{
    num::NonZeroUsize,
    task::{Context, Poll},
    time::Duration,
};
use std::collections::HashSet;

use futures::{stream::pending, task::noop_waker_ref, StreamExt as _};
use rand::SeedableRng as _;
use rand_chacha::ChaCha20Rng;
use tokio_stream::iter;

use crate::{
    cover_traffic_2::SessionCoverTraffic,
    message::OutboundMessage,
    message_scheduler::{
        round_info::RoundInfo, session_info::SessionInfo, MessageScheduler, Settings,
    },
    release_delayer::SessionProcessedMessageDelayer,
};

#[tokio::test]
async fn no_substream_ready() {
    let rng = ChaCha20Rng::from_entropy();
    let mut scheduler = MessageScheduler::with_test_values(
        // Round `1` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter([0]).map(Into::into)),
            HashSet::from_iter([1u128.into()]),
            0,
        ),
        // Round `1` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroUsize::try_from(1).unwrap(),
            1u128.into(),
            rng,
            Box::new(iter([0]).map(Into::into)),
            vec![],
        ),
        // Round clock (same as above)
        Box::new(iter([0]).map(Into::into)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        // Not used in tests
        Settings {
            additional_safety_intervals: 0,
            expected_intervals_per_session: NonZeroUsize::try_from(1).unwrap(),
            maximum_release_delay_in_rounds: NonZeroUsize::try_from(1).unwrap(),
            round_duration: Duration::from_millis(100),
            rounds_per_interval: NonZeroUsize::try_from(1).unwrap(),
        },
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // We poll for round 0, which returns `Pending`, as per the default scheduler
    // configuration.
    assert!(scheduler.poll_next_unpin(&mut cx).is_pending());
}

#[tokio::test]
async fn cover_traffic_substream_ready() {
    let rng = ChaCha20Rng::from_entropy();
    let mut scheduler = MessageScheduler::with_test_values(
        // Round `0` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter([0]).map(Into::into)),
            HashSet::from_iter([0u128.into()]),
            0,
        ),
        // Round `1` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroUsize::try_from(1).unwrap(),
            1u128.into(),
            rng,
            Box::new(iter([0]).map(Into::into)),
            vec![],
        ),
        // Round clock (same as above)
        Box::new(iter([0]).map(Into::into)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        // Not used in tests
        Settings {
            additional_safety_intervals: 0,
            expected_intervals_per_session: NonZeroUsize::try_from(1).unwrap(),
            maximum_release_delay_in_rounds: NonZeroUsize::try_from(1).unwrap(),
            round_duration: Duration::from_millis(100),
            rounds_per_interval: NonZeroUsize::try_from(1).unwrap(),
        },
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return a cover message.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            cover_message: Some(vec![].into()),
            processed_messages: vec![]
        }))
    );
}

#[tokio::test]
async fn release_delayer_substream_ready() {
    let rng = ChaCha20Rng::from_entropy();
    let mut scheduler = MessageScheduler::with_test_values(
        // Round `1` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter([0]).map(Into::into)),
            HashSet::from_iter([1u128.into()]),
            0,
        ),
        // Round `0` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroUsize::try_from(1).unwrap(),
            0u128.into(),
            rng,
            Box::new(iter([0]).map(Into::into)),
            vec![OutboundMessage::from(b"test".to_vec())],
        ),
        // Round clock (same as above)
        Box::new(iter([0]).map(Into::into)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        // Not used in tests
        Settings {
            additional_safety_intervals: 0,
            expected_intervals_per_session: NonZeroUsize::try_from(1).unwrap(),
            maximum_release_delay_in_rounds: NonZeroUsize::try_from(1).unwrap(),
            round_duration: Duration::from_millis(100),
            rounds_per_interval: NonZeroUsize::try_from(1).unwrap(),
        },
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return the processed messages.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            cover_message: None,
            processed_messages: vec![OutboundMessage::from(b"test".to_vec())]
        }))
    );
}

#[tokio::test]
async fn both_substreams_ready() {
    let rng = ChaCha20Rng::from_entropy();
    let mut scheduler = MessageScheduler::with_test_values(
        // Round `0` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter([0]).map(Into::into)),
            HashSet::from_iter([0u128.into()]),
            0,
        ),
        // Round `0` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroUsize::try_from(1).unwrap(),
            0u128.into(),
            rng,
            Box::new(iter([0]).map(Into::into)),
            vec![OutboundMessage::from(b"test".to_vec())],
        ),
        // Round clock (same as above)
        Box::new(iter([0]).map(Into::into)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        // Not used in tests
        Settings {
            additional_safety_intervals: 0,
            expected_intervals_per_session: NonZeroUsize::try_from(1).unwrap(),
            maximum_release_delay_in_rounds: NonZeroUsize::try_from(1).unwrap(),
            round_duration: Duration::from_millis(100),
            rounds_per_interval: NonZeroUsize::try_from(1).unwrap(),
        },
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return the processed messages and a cover
    // message.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            cover_message: Some(vec![].into()),
            processed_messages: vec![OutboundMessage::from(b"test".to_vec())]
        }))
    );
}

#[tokio::test]
async fn round_change() {
    let rng = ChaCha20Rng::from_entropy();
    let mut scheduler = MessageScheduler::with_test_values(
        // Round `1` scheduled, tick will yield round `0` then round `1`, then round `2`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter([0, 1, 2]).map(Into::into)),
            HashSet::from_iter([1u128.into()]),
            0,
        ),
        // Round `2` scheduled, tick will yield round `0` then round `1`, then round `2`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroUsize::try_from(1).unwrap(),
            2u128.into(),
            rng,
            Box::new(iter([0, 1, 2]).map(Into::into)),
            vec![OutboundMessage::from(b"test".to_vec())],
        ),
        // Round clock (same as above)
        Box::new(iter([0, 1, 2]).map(Into::into)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        // Not used in tests
        Settings {
            additional_safety_intervals: 0,
            expected_intervals_per_session: NonZeroUsize::try_from(1).unwrap(),
            maximum_release_delay_in_rounds: NonZeroUsize::try_from(1).unwrap(),
            round_duration: Duration::from_millis(100),
            rounds_per_interval: NonZeroUsize::try_from(1).unwrap(),
        },
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round `0`, which should return `Pending`.
    assert_eq!(scheduler.poll_next_unpin(&mut cx), Poll::Pending);

    // Poll for round `1`, which should return a cover message.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            cover_message: Some(vec![].into()),
            processed_messages: vec![]
        }))
    );

    // Poll for round `2`, which should return the processed messages.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            cover_message: None,
            processed_messages: vec![OutboundMessage::from(b"test".to_vec())]
        }))
    );
}

#[tokio::test]
async fn session_change() {
    let rng = ChaCha20Rng::from_entropy();
    let mut scheduler = MessageScheduler::with_test_values(
        SessionCoverTraffic::with_test_values(
            Box::new(iter([0]).map(Into::into)),
            HashSet::from_iter([1u128.into()]),
            // One data message to process.
            1,
        ),
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroUsize::try_from(1).unwrap(),
            2u128.into(),
            rng,
            Box::new(iter([0]).map(Into::into)),
            // One unreleased message.
            vec![b"test".to_vec().into()],
        ),
        // Round clock (same as above)
        Box::new(iter([0]).map(Into::into)),
        // Session clock
        Box::new(iter([SessionInfo {
            core_quota: 1,
            session_number: 0u128.into(),
        }])),
        // Not used in tests
        Settings {
            additional_safety_intervals: 0,
            expected_intervals_per_session: NonZeroUsize::try_from(1).unwrap(),
            maximum_release_delay_in_rounds: NonZeroUsize::try_from(1).unwrap(),
            round_duration: Duration::from_millis(100),
            rounds_per_interval: NonZeroUsize::try_from(1).unwrap(),
        },
    );
    assert_eq!(scheduler.cover_traffic.unprocessed_data_messages(), 1);
    assert_eq!(scheduler.release_delayer.unreleased_messages().len(), 1);
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll after new session. All the sub-streams should be reset.
    let _ = scheduler.poll_next_unpin(&mut cx);
    assert_eq!(scheduler.cover_traffic.unprocessed_data_messages(), 0);
    assert_eq!(scheduler.release_delayer.unreleased_messages().len(), 0);
}
