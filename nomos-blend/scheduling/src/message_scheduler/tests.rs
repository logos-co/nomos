use core::{
    num::NonZeroU64,
    task::{Context, Poll},
};
use std::collections::HashSet;

use futures::{StreamExt as _, stream::pending, task::noop_waker_ref};
use nomos_utils::blake_rng::BlakeRng;
use rand::SeedableRng as _;
use tokio_stream::iter;

use crate::{
    cover_traffic::SessionCoverTraffic,
    message_scheduler::{
        MessageScheduler,
        round_info::{Round, RoundInfo, RoundReleaseType},
        session_info::SessionInfo,
    },
    release_delayer::SessionProcessedMessageDelayer,
};

#[tokio::test]
async fn no_substream_ready_and_no_data_messages() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::<_, _, (), ()>::with_test_values(
        // Round `1` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([1u128.into()]),
            rng.clone(),
            0,
        ),
        // Round `1` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroU64::try_from(1).unwrap(),
            1u128.into(),
            rng,
            Box::new(iter(rounds)),
            vec![],
        ),
        // Round clock (same as above)
        Box::new(iter(rounds)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        vec![],
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // We poll for round 0, which returns `Pending`, as per the default scheduler
    // configuration.
    assert!(scheduler.poll_next_unpin(&mut cx).is_pending());
}

#[tokio::test]
async fn no_substream_ready_with_data_messages() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::<_, _, (), u32>::with_test_values(
        // Round `1` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([1u128.into()]),
            rng.clone(),
            0,
        ),
        // Round `1` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroU64::try_from(1).unwrap(),
            1u128.into(),
            rng,
            Box::new(iter(rounds)),
            vec![],
        ),
        // Round clock (same as above)
        Box::new(iter(rounds)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        vec![1, 2],
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // We poll for round 0, which returns `Ready` since we have data messages to
    // return.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            data_messages: vec![1, 2],
            release_type: None
        }))
    );
    // We test that the released data messages have been removed from the queue.
    assert!(scheduler.data_messages.is_empty());
}

#[tokio::test]
async fn cover_traffic_substream_ready() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::<_, _, (), u32>::with_test_values(
        // Round `0` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([0u128.into()]),
            rng.clone(),
            0,
        ),
        // Round `1` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroU64::try_from(1).unwrap(),
            1u128.into(),
            rng,
            Box::new(iter(rounds)),
            vec![],
        ),
        // Round clock (same as above)
        Box::new(iter(rounds)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        vec![1],
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return a cover message.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            data_messages: vec![1],
            release_type: Some(RoundReleaseType::OnlyCoverMessage)
        }))
    );
}

#[tokio::test]
async fn release_delayer_substream_ready() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::<_, _, u32, u32>::with_test_values(
        // Round `1` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([1u128.into()]),
            rng.clone(),
            0,
        ),
        // Round `0` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroU64::try_from(1).unwrap(),
            0u128.into(),
            rng,
            Box::new(iter(rounds)),
            vec![1],
        ),
        // Round clock (same as above)
        Box::new(iter(rounds)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        vec![2],
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return the processed messages.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            data_messages: vec![2],
            release_type: Some(RoundReleaseType::OnlyProcessedMessages(vec![1]))
        }))
    );
}

#[tokio::test]
async fn both_substreams_ready() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::<_, _, u32, ()>::with_test_values(
        // Round `0` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([0u128.into()]),
            rng.clone(),
            0,
        ),
        // Round `0` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroU64::try_from(1).unwrap(),
            0u128.into(),
            rng,
            Box::new(iter(rounds)),
            vec![1],
        ),
        // Round clock (same as above)
        Box::new(iter(rounds)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        vec![],
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return the processed messages and a cover
    // message.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            data_messages: vec![],
            release_type: Some(RoundReleaseType::ProcessedAndCoverMessages(vec![1]))
        }))
    );
}

#[tokio::test]
async fn round_change() {
    let rng = BlakeRng::from_entropy();
    let rounds = [
        Round::from(0),
        Round::from(1),
        Round::from(2),
        Round::from(3),
    ];
    let mut scheduler = MessageScheduler::<_, _, (), u32>::with_test_values(
        // Round `1` scheduled, tick will yield round `0` then round `1`, then round `2`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([1u128.into(), 3u128.into()]),
            rng.clone(),
            0,
        ),
        // Round `2` scheduled, tick will yield round `0` then round `1`, then round `2`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroU64::try_from(1).unwrap(),
            2u128.into(),
            rng,
            Box::new(iter(rounds)),
            vec![()],
        ),
        // Round clock (same as above)
        Box::new(iter(rounds)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
        vec![],
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round `0`, which should return `Pending`.
    assert_eq!(scheduler.poll_next_unpin(&mut cx), Poll::Pending);

    // Poll for round `1`, which should return a cover message.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            data_messages: vec![],
            release_type: Some(RoundReleaseType::OnlyCoverMessage)
        }))
    );
    assert!(scheduler.data_messages.is_empty());

    scheduler.queue_data_message(2);
    // Poll for round `2`, which should return the processed messages and the queued
    // data message.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            data_messages: vec![2],
            release_type: Some(RoundReleaseType::OnlyProcessedMessages(vec![()]))
        }))
    );
    assert!(scheduler.data_messages.is_empty());

    scheduler.queue_data_message(3);
    // Poll for round `3`, which should skip the cover message (even if scheduled)
    // and should only return the queued data message instead.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            data_messages: vec![3],
            release_type: None
        }))
    );
    assert!(scheduler.data_messages.is_empty());
}

#[tokio::test]
async fn session_change() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::with_test_values(
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([1u128.into()]),
            rng.clone(),
            // One data message to process.
            1,
        ),
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroU64::try_from(1).unwrap(),
            2u128.into(),
            rng,
            Box::new(iter(rounds)),
            // One unreleased message.
            vec![()],
        ),
        // Round clock (same as above)
        Box::new(iter(rounds)),
        // Session clock
        Box::new(iter([SessionInfo {
            core_quota: 1,
            session_number: 0u128.into(),
        }])),
        vec![()],
    );
    assert_eq!(scheduler.cover_traffic.unprocessed_data_messages(), 1);
    assert_eq!(scheduler.release_delayer.unreleased_messages().len(), 1);
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll after new session. All the sub-streams should be reset, and queued data
    // messages deleted.
    let _ = scheduler.poll_next_unpin(&mut cx);
    assert_eq!(scheduler.cover_traffic.unprocessed_data_messages(), 0);
    assert_eq!(scheduler.release_delayer.unreleased_messages().len(), 0);
    assert!(scheduler.data_messages.is_empty());
}
