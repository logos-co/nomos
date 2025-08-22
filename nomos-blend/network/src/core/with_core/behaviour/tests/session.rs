use futures::StreamExt as _;
use libp2p::{Multiaddr, PeerId};
use libp2p_swarm_test::SwarmExt as _;
use nomos_blend_message::crypto::Ed25519PrivateKey;
use nomos_blend_scheduling::membership::{Membership, Node};
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::select;

use crate::core::{
    tests::utils::{TestEncapsulatedMessage, TestSwarm},
    with_core::{
        behaviour::{
            old_session,
            tests::utils::{
                build_memberships, BehaviourBuilder, SwarmExt as _, TestBehaviourEvent,
            },
            Event,
        },
        error::Error,
    },
};

#[test(tokio::test)]
async fn publish_message_after_new_session_started() {
    let mut dialer = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut listener = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());

    listener.listen().with_memory_addr_external().await;
    dialer
        .connect_and_wait_for_outbound_upgrade(&mut listener)
        .await;

    // Start a new session before sending any message through the connection.
    let memberships = build_memberships(&[&dialer, &listener]);
    dialer
        .behaviour_mut()
        .current_session
        .start_new_session(memberships[0].clone());
    listener
        .behaviour_mut()
        .current_session
        .start_new_session(memberships[1].clone());

    // Send a message but expect [`Error::NoPeers`]
    // because we haven't establish connections for the new session.
    let test_message = TestEncapsulatedMessage::new(b"msg");
    let result = dialer
        .behaviour_mut()
        .current_session
        .validate_and_publish_message(test_message.clone());
    assert_eq!(result, Err(Error::NoPeers));

    // Establish a connection for the new session.
    dialer
        .connect_and_wait_for_outbound_upgrade(&mut listener)
        .await;

    // Now we can send the message successfully.
    dialer
        .behaviour_mut()
        .current_session
        .validate_and_publish_message(test_message.clone())
        .unwrap();
    loop {
        select! {
            _ = dialer.select_next_some() => {}
            event = listener.select_next_some() => {
                if let SwarmEvent::Behaviour(TestBehaviourEvent::CurrentSession(Event::Message(message, _))) = event {
                    assert_eq!(message.id(), test_message.id());
                    break;
                }
            }
        }
    }
}

#[test(tokio::test)]
async fn forward_message_in_old_session() {
    let mut sender = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut forwarder = TestSwarm::new(|id| {
        BehaviourBuilder::default()
            .with_identity(id)
            .with_peering_degree(2..=2)
            .build()
    });
    let mut receiver = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());

    forwarder.listen().with_memory_addr_external().await;
    receiver.listen().with_memory_addr_external().await;

    // Connect nodes: Sender -> Forwarder -> Receiver
    sender
        .connect_and_wait_for_outbound_upgrade(&mut forwarder)
        .await;
    forwarder
        .connect_and_wait_for_outbound_upgrade(&mut receiver)
        .await;

    // Before sending any message, start a new session only for the forwarder and receiver.
    // Then, the topology looks like:
    // - Old session: Sender -> Forwarder -> Receiver
    // - New session: No connections yet
    let memberships = build_memberships(&[&sender, &forwarder, &receiver]);
    let (peers, msg_ids) = forwarder
        .behaviour_mut()
        .current_session
        .start_new_session(memberships[1].clone());
    forwarder.behaviour_mut().old_session.start(
        peers
            .into_iter()
            .map(|(peer_id, details)| (peer_id, details.connection_id))
            .collect(),
        msg_ids,
    );
    let (peers, msg_ids) = receiver
        .behaviour_mut()
        .current_session
        .start_new_session(memberships[2].clone());
    receiver.behaviour_mut().old_session.start(
        peers
            .into_iter()
            .map(|(peer_id, details)| (peer_id, details.connection_id))
            .collect(),
        msg_ids,
    );

    // The sender publishes a message to the forwarder.
    // Publishing messages always uses the current session.
    // That's why we didn't start a new session for the sender yet.
    let test_message = TestEncapsulatedMessage::new(b"msg");
    sender
        .behaviour_mut()
        .current_session
        .validate_and_publish_message(test_message.clone())
        .unwrap();
    println!("Message published");
    // We expect that the message goes through the forwarder and receiver.
    loop {
        select! {
            _ = sender.select_next_some() => {}
            event = forwarder.select_next_some() => {
                // println!("Forwarder event: {:?}", event);
                if let SwarmEvent::Behaviour(TestBehaviourEvent::OldSession(old_session::Event::Message(message, peer))) = event {
                    assert_eq!(message.id(), test_message.id());
                    forwarder.behaviour_mut().old_session
                        .forward_validated_message(&message, peer)
                        .unwrap();
                }
            }
            event = receiver.select_next_some() => {
                // println!("Receiver event: {:?}", event);
                if let SwarmEvent::Behaviour(TestBehaviourEvent::OldSession(old_session::Event::Message(message, _))) = event {
                    assert_eq!(message.id(), test_message.id());
                    break;
                }
            }
        }
    }
}

// #[test(tokio::test)]
// async fn close_old_session() {
//     let mut dialer = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
//     let mut listener = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());

//     listener.listen().with_memory_addr_external().await;
//     dialer
//         .connect_and_wait_for_outbound_upgrade(&mut listener)
//         .await;

//     // Start a new session.
//     let memberships = build_memberships(&[*dialer.local_peer_id(), *listener.local_peer_id()]);
//     dialer
//         .behaviour_mut()
//         .start_new_session(memberships[0].clone());
//     listener
//         .behaviour_mut()
//         .start_new_session(memberships[1].clone());

//     // Finish the transition period
//     dialer.behaviour_mut().close_old_session();
//     listener.behaviour_mut().close_old_session();

//     // Expect that the connection is closed after 10s (default swarm timeout).
//     loop {
//         select! {
//             _ = dialer.select_next_some() => {}
//             event = listener.select_next_some() => {
//                 if let SwarmEvent::ConnectionClosed { .. } = event {
//                     assert!(listener.behaviour_mut().old_session.is_none());
//                     break;
//                 }
//             }
//         }
//     }
// }
