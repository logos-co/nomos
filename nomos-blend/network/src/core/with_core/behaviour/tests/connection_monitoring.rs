use core::time::Duration;

use futures::StreamExt as _;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::{select, time::sleep};

use crate::core::with_core::behaviour::{
    tests::utils::{IntervalProvider, TestEncapsulatedMessage, TestSwarm},
    Behaviour, Event, NegotiatedPeerState,
};

#[test(tokio::test)]
async fn detect_spammy_peer() {
    let mut dialing_swarm = TestSwarm::new(Behaviour::with_interval_provider(
        IntervalProvider::new(Duration::from_secs(2)),
    ));
    let mut listening_swarm = TestSwarm::new(Behaviour::with_interval_provider(
        IntervalProvider::new(Duration::from_secs(2)),
    ));

    let listening_address = listening_swarm.start_listening().await;

    dialing_swarm.dial(listening_address).unwrap();

    loop {
        select! {
            dialing_event = dialing_swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::OutboundConnectionUpgradeSucceeded(peer_id)) = dialing_event {
                    assert_eq!(peer_id, *listening_swarm.local_peer_id());
                    break;
                }
            }
            _ = listening_swarm.select_next_some() => {}
        }
    }

    // Send two messages when only one was expected.
    dialing_swarm
        .behaviour_mut()
        .validate_and_publish_message(TestEncapsulatedMessage::new(b"msg1").into_inner())
        .unwrap();
    dialing_swarm
        .behaviour_mut()
        .validate_and_publish_message(TestEncapsulatedMessage::new(b"msg2").into_inner())
        .unwrap();

    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_event = listening_swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, NegotiatedPeerState::Spammy)) = listening_event {
                    assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                    assert!(listening_swarm.behaviour().negotiated_peers.is_empty());
                    break;
                }
            }
        }
    }
}

#[test(tokio::test)]
async fn detect_unhealthy_peer() {
    let mut dialing_swarm = TestSwarm::new(Behaviour::with_interval_provider(
        IntervalProvider::new(Duration::from_secs(2)),
    ));
    let mut listening_swarm = TestSwarm::new(Behaviour::with_interval_provider(
        IntervalProvider::new(Duration::from_secs(2)),
    ));

    let listening_address = listening_swarm.start_listening().await;

    dialing_swarm.dial(listening_address).unwrap();

    // Do not send any message from dialing to listening swarm.

    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_event = listening_swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::UnhealthyPeer(peer_id)) = listening_event {
                    assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                    assert!(listening_swarm.behaviour().negotiated_peers.get(&peer_id).unwrap().negotiated_state.is_unhealthy());
                    break;
                }
            }
        }
    }

    // We make sure that the same "Unhealthy" notification is not bubbled up to the
    // swarm again by the behaviour for an already unhealthy peer.

    loop {
        select! {
            () = sleep(Duration::from_secs(3)) => {
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
}

#[test(tokio::test)]
async fn restore_healthy_peer() {
    let mut dialing_swarm = TestSwarm::new(Behaviour::with_interval_provider(
        IntervalProvider::new(Duration::from_secs(2)),
    ));
    let mut listening_swarm = TestSwarm::new(Behaviour::with_interval_provider(
        IntervalProvider::new(Duration::from_secs(2)),
    ));

    let listening_address = listening_swarm.start_listening().await;

    dialing_swarm.dial(listening_address).unwrap();

    // Wait for connection to be upgraded.
    loop {
        select! {
            dialing_event = dialing_swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::OutboundConnectionUpgradeSucceeded(peer_id)) = dialing_event {
                    assert_eq!(peer_id, *listening_swarm.local_peer_id());
                    break;
                }
            }
            _ = listening_swarm.select_next_some() => {}
        }
    }

    // Modify storage to mark peer as unhealthy.
    listening_swarm
        .behaviour_mut()
        .negotiated_peers
        .get_mut(dialing_swarm.local_peer_id())
        .unwrap()
        .negotiated_state = NegotiatedPeerState::Unhealthy;

    // Send a message to the listening swarm to revert from unhealthy to healthy.
    dialing_swarm
        .behaviour_mut()
        .validate_and_publish_message(TestEncapsulatedMessage::new(b"msg").into_inner())
        .unwrap();

    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_event = listening_swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::HealthyPeer(peer_id)) = listening_event {
                    assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                    assert!(listening_swarm.behaviour().negotiated_peers.get(&peer_id).unwrap().negotiated_state.is_healthy());
                    break;
                }
            }
        }
    }
}
