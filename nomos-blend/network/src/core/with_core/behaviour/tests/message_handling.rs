use core::time::Duration;
use std::collections::HashSet;

use futures::StreamExt as _;
use libp2p_swarm_test::SwarmExt as _;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::{select, time::sleep};

use crate::{
    core::with_core::{
        behaviour::{
            tests::utils::{BehaviourBuilder, SwarmExt as _, TestEncapsulatedMessage, TestSwarm},
            Event, NegotiatedPeerState, SpamReason,
        },
        error::Error,
    },
    message::ValidateMessagePublicHeader as _,
};

#[test(tokio::test)]
async fn message_sending_and_reception() {
    let mut dialing_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut listening_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());

    listening_swarm.listen().with_memory_addr_external().await;
    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

    // Send one message, which is within the range of expected messages.
    let test_message = TestEncapsulatedMessage::new(b"msg");
    let test_message_id = test_message.id();
    dialing_swarm
        .behaviour_mut()
        .validate_and_publish_message(test_message.clone())
        .unwrap();

    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_event = listening_swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::Message(encapsulated_message, peer_id)) = listening_event {
                    assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                    assert_eq!(*encapsulated_message, test_message.clone().validate_public_header().unwrap());
                    break;
                }
            }
        }
    }

    assert_eq!(
        dialing_swarm
            .behaviour()
            .exchanged_message_identifiers
            .get(listening_swarm.local_peer_id())
            .unwrap(),
        &vec![test_message_id].into_iter().collect::<HashSet<_>>()
    );
}

#[test(tokio::test)]
async fn invalid_public_header_message_publish() {
    let mut dialing_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());

    let invalid_signature_message = TestEncapsulatedMessage::new_with_invalid_signature(b"data");
    assert_eq!(
        dialing_swarm
            .behaviour_mut()
            .validate_and_publish_message(invalid_signature_message.into_inner()),
        Err(Error::InvalidMessage)
    );
}

#[test(tokio::test)]
async fn undeserializable_message_received() {
    let mut dialing_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut listening_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());

    listening_swarm.listen().with_memory_addr_external().await;
    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

    dialing_swarm
        .behaviour_mut()
        .force_send_serialized_message_to_peer(b"msg".to_vec(), *listening_swarm.local_peer_id())
        .unwrap();

    let mut events_to_match = 2u8;
    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_swarm_event = listening_swarm.select_next_some() => {
                match listening_swarm_event {
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, peer_state)) => {
                        assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                        assert_eq!(peer_state, NegotiatedPeerState::Spammy(SpamReason::UndeserializableMessage));
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
async fn duplicate_message_received() {
    let mut dialing_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut listening_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());

    listening_swarm.listen().with_memory_addr_external().await;
    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

    let test_message = TestEncapsulatedMessage::new(b"msg");
    dialing_swarm
        .behaviour_mut()
        .validate_and_publish_message(test_message.clone())
        .unwrap();

    // Wait enough time to not considered spammy by the listener.
    sleep(Duration::from_secs(3)).await;

    // This is a duplicate message, so the listener will mark the dialer as spammy.
    dialing_swarm
        .behaviour_mut()
        .force_send_message_to_peer(&test_message, *listening_swarm.local_peer_id())
        .unwrap();

    let mut events_to_match = 2u8;
    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_swarm_event = listening_swarm.select_next_some() => {
                match listening_swarm_event {
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, peer_state)) => {
                        assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                        assert_eq!(peer_state, NegotiatedPeerState::Spammy(SpamReason::DuplicateMessage));
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
async fn invalid_public_header_message_received() {
    let mut dialing_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut listening_swarm =
        TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());

    listening_swarm.listen().with_memory_addr_external().await;
    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

    let invalid_public_header_message = TestEncapsulatedMessage::new_with_invalid_signature(b"");
    dialing_swarm
        .behaviour_mut()
        .force_send_message_to_peer(
            &invalid_public_header_message,
            *listening_swarm.local_peer_id(),
        )
        .unwrap();

    let mut events_to_match = 2u8;
    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_swarm_event = listening_swarm.select_next_some() => {
                match listening_swarm_event {
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, peer_state)) => {
                        assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                        assert_eq!(peer_state, NegotiatedPeerState::Spammy(SpamReason::InvalidPublicHeader));
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
