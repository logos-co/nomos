use core::time::Duration;

use futures::StreamExt as _;
use libp2p_swarm_test::SwarmExt as _;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::{select, time::sleep};

use crate::core::with_core::behaviour::{
    tests::utils::{
        BehaviourBuilder, IntervalProviderBuilder, SwarmExt as _, TestEncapsulatedMessage,
        TestSwarm,
    },
    Event, NegotiatedPeerState, SpamReason,
};

#[test(tokio::test)]
async fn detect_spammy_peer() {
    let mut dialing_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut listening_swarm = TestSwarm::new(|id| {
        BehaviourBuilder::default()
            .with_identity(id)
            .with_provider(IntervalProviderBuilder::default().with_range(1..=1).build())
            .build()
    });

    listening_swarm.listen().with_memory_addr_external().await;
    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

    // We let the first observation clock tick.
    sleep(Duration::from_secs(2)).await;

    // Send two messages when only one was expected.
    dialing_swarm
        .behaviour_mut()
        .validate_and_publish_message(TestEncapsulatedMessage::new(b"msg1").into_inner())
        .unwrap();
    dialing_swarm
        .behaviour_mut()
        .validate_and_publish_message(TestEncapsulatedMessage::new(b"msg2").into_inner())
        .unwrap();

    let mut events_to_match = 2u8;
    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_event = listening_swarm.select_next_some() => {
                match listening_event {
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, NegotiatedPeerState::Spammy(SpamReason::TooManyMessages))) => {
                        assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                        assert!(listening_swarm.behaviour().negotiated_peers.is_empty());
                        events_to_match -= 1;
                    }
                    SwarmEvent::ConnectionClosed { peer_id, endpoint, .. } => {
                        assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                        assert!(endpoint.is_listener());
                        events_to_match -= 1;
                    }
                    _ => {}
                }
            }
        }
        if events_to_match == 0 {
            break;
        }
    }
}

#[test(tokio::test)]
async fn detect_unhealthy_peer() {
    let mut dialing_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut listening_swarm = TestSwarm::new(|id| {
        BehaviourBuilder::default()
            .with_identity(id)
            .with_provider(IntervalProviderBuilder::default().with_range(1..=1).build())
            .build()
    });

    listening_swarm.listen().with_memory_addr_external().await;
    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

    // Do not send any message from dialing to listening swarm.

    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_event = listening_swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::UnhealthyPeer(peer_id)) = listening_event {
                    assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                    break;
                }
            }
        }
    }

    // We make sure that the same "Unhealthy" notification is not bubbled up to the
    // swarm again by the behaviour for an already unhealthy peer.

    loop {
        select! {
            () = sleep(Duration::from_secs(5)) => {
                break;
            }
            _ = dialing_swarm.select_next_some() => {}
            listening_event = listening_swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::UnhealthyPeer(peer_id)) = listening_event {
                    assert!(peer_id != *dialing_swarm.local_peer_id());
                }
            }
        }
    }

    assert!(listening_swarm
        .behaviour()
        .negotiated_peers
        .get(dialing_swarm.local_peer_id())
        .unwrap()
        .negotiated_state
        .is_unhealthy());
}

#[test(tokio::test)]
async fn restore_healthy_peer() {
    let mut dialing_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut listening_swarm = TestSwarm::new(|id| {
        BehaviourBuilder::default()
            .with_identity(id)
            .with_provider(IntervalProviderBuilder::default().with_range(1..=1).build())
            .build()
    });

    listening_swarm.listen().with_memory_addr_external().await;
    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

    // Let the connection turn unhealthy.
    sleep(Duration::from_secs(4)).await;

    // Send a message to the listening swarm to revert from unhealthy to healthy.
    dialing_swarm
        .behaviour_mut()
        .force_send_message_to_peer(
            &TestEncapsulatedMessage::new(b"msg"),
            *listening_swarm.local_peer_id(),
        )
        .unwrap();

    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_event = listening_swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::HealthyPeer(peer_id)) = listening_event {
                    assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                    break;
                }
            }
        }
    }

    assert!(listening_swarm
        .behaviour()
        .negotiated_peers
        .get(dialing_swarm.local_peer_id())
        .unwrap()
        .negotiated_state
        .is_healthy());
}
