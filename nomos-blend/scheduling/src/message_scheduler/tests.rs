use core::{
    num::NonZeroU64,
    task::{Context, Poll},
};
use std::collections::HashSet;

use futures::{stream::pending, task::noop_waker_ref, StreamExt as _};
use nomos_utils::blake_rng::BlakeRng;
use rand::SeedableRng as _;
use tokio_stream::iter;

use crate::{
    cover_traffic::SessionCoverTraffic,
    message_scheduler::{
        round_info::{Round, RoundInfo},
        session_info::SessionInfo,
        MessageScheduler,
    },
    release_delayer::SessionProcessedMessageDelayer,
};

#[tokio::test]
async fn no_substream_ready() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::<_, _, ()>::with_test_values(
        // Round `1` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([1u128.into()]),
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
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // We poll for round 0, which returns `Pending`, as per the default scheduler
    // configuration.
    assert!(scheduler.poll_next_unpin(&mut cx).is_pending());
}

#[tokio::test]
async fn cover_traffic_substream_ready() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::<_, _, ()>::with_test_values(
        // Round `0` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([0u128.into()]),
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
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return a cover message.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            cover_message_generation_flag: Some(()),
            processed_messages: vec![]
        }))
    );
}

#[tokio::test]
async fn release_delayer_substream_ready() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::<_, _, ()>::with_test_values(
        // Round `1` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([1u128.into()]),
            0,
        ),
        // Round `0` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroU64::try_from(1).unwrap(),
            0u128.into(),
            rng,
            Box::new(iter(rounds)),
            vec![()],
        ),
        // Round clock (same as above)
        Box::new(iter(rounds)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return the processed messages.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            cover_message_generation_flag: None,
            processed_messages: vec![()]
        }))
    );
}

#[tokio::test]
async fn both_substreams_ready() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::<_, _, ()>::with_test_values(
        // Round `0` scheduled, tick will yield round `0`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([0u128.into()]),
            0,
        ),
        // Round `0` scheduled, tick will yield round `0`.
        SessionProcessedMessageDelayer::with_test_values(
            NonZeroU64::try_from(1).unwrap(),
            0u128.into(),
            rng,
            Box::new(iter(rounds)),
            vec![()],
        ),
        // Round clock (same as above)
        Box::new(iter(rounds)),
        // Session clock. Pending so we don't overwrite the test setup.
        Box::new(pending()),
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round 0, which should return the processed messages and a cover
    // message.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            cover_message_generation_flag: Some(()),
            processed_messages: vec![()]
        }))
    );
}

#[tokio::test]
async fn round_change() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0), Round::from(1), Round::from(2)];
    let mut scheduler = MessageScheduler::<_, _, ()>::with_test_values(
        // Round `1` scheduled, tick will yield round `0` then round `1`, then round `2`.
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([1u128.into()]),
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
    );
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll for round `0`, which should return `Pending`.
    assert_eq!(scheduler.poll_next_unpin(&mut cx), Poll::Pending);

    // Poll for round `1`, which should return a cover message.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            cover_message_generation_flag: Some(()),
            processed_messages: vec![]
        }))
    );

    // Poll for round `2`, which should return the processed messages.
    assert_eq!(
        scheduler.poll_next_unpin(&mut cx),
        Poll::Ready(Some(RoundInfo {
            cover_message_generation_flag: None,
            processed_messages: vec![()]
        }))
    );
}

#[tokio::test]
async fn session_change() {
    let rng = BlakeRng::from_entropy();
    let rounds = [Round::from(0)];
    let mut scheduler = MessageScheduler::with_test_values(
        SessionCoverTraffic::with_test_values(
            Box::new(iter(rounds)),
            HashSet::from_iter([1u128.into()]),
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
    );
    assert_eq!(scheduler.cover_traffic.unprocessed_data_messages(), 1);
    assert_eq!(scheduler.release_delayer.unreleased_messages().len(), 1);
    let mut cx = Context::from_waker(noop_waker_ref());

    // Poll after new session. All the sub-streams should be reset.
    let _ = scheduler.poll_next_unpin(&mut cx);
    assert_eq!(scheduler.cover_traffic.unprocessed_data_messages(), 0);
    assert_eq!(scheduler.release_delayer.unreleased_messages().len(), 0);
}
