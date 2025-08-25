use futures::StreamExt as _;
use libp2p_swarm_test::SwarmExt as _;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::select;

use crate::core::{
    tests::utils::{TestEncapsulatedMessage, TestSwarm},
    with_core::{
        behaviour::{
            tests::utils::{build_memberships, BehaviourBuilder, SwarmExt as _},
            Event,
        },
        error::Error,
    },
};

#[test(tokio::test)]
async fn publish_message() {
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
        .start_new_session(memberships[0].clone());
    listener
        .behaviour_mut()
        .start_new_session(memberships[1].clone());

    // Send a message but expect [`Error::NoPeers`]
    // because we haven't establish connections for the new session.
    let test_message = TestEncapsulatedMessage::new(b"msg");
    let result = dialer
        .behaviour_mut()
        .validate_and_publish_message(test_message.clone());
    assert_eq!(result, Err(Error::NoPeers));

    // Establish a connection for the new session.
    dialer
        .connect_and_wait_for_outbound_upgrade(&mut listener)
        .await;

    // Now we can send the message successfully.
    dialer
        .behaviour_mut()
        .validate_and_publish_message(test_message.clone())
        .unwrap();
    loop {
        select! {
            _ = dialer.select_next_some() => {}
            event = listener.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::Message(message, _)) = event {
                    assert_eq!(message.id(), test_message.id());
                    break;
                }
            }
        }
    }
}

#[test(tokio::test)]
async fn forward_message() {
    let mut sender = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut forwarder = TestSwarm::new(|id| {
        BehaviourBuilder::default()
            .with_identity(id)
            .with_peering_degree(2..=2)
            .build()
    });
    let mut receiver1 = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut receiver2 = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());

    forwarder.listen().with_memory_addr_external().await;
    receiver1.listen().with_memory_addr_external().await;
    receiver2.listen().with_memory_addr_external().await;

    // Connect 3 nodes: sender -> forwarder -> receiver1
    sender
        .connect_and_wait_for_outbound_upgrade(&mut forwarder)
        .await;
    forwarder
        .connect_and_wait_for_outbound_upgrade(&mut receiver1)
        .await;

    // Before sending any message, start a new session
    // only for the forwarder, receiver1, and receiver2.
    // And, connect the forwarder to the receiver2 for the new session.
    // Then, the topology looks like:
    // - Old session: sender -> forwarder -> receiver1
    // - New session:           forwarder -> receiver2
    let memberships = build_memberships(&[&sender, &forwarder, &receiver1, &receiver2]);
    forwarder
        .behaviour_mut()
        .start_new_session(memberships[1].clone());
    receiver1
        .behaviour_mut()
        .start_new_session(memberships[2].clone());
    receiver2
        .behaviour_mut()
        .start_new_session(memberships[3].clone());
    forwarder
        .connect_and_wait_for_outbound_upgrade(&mut receiver2)
        .await;

    // The sender publishes a message to the forwarder.
    // Publishing messages always uses the current session.
    // It should work because the sender doesn't know about the new session yet.
    let test_message = TestEncapsulatedMessage::new(b"msg");
    sender
        .behaviour_mut()
        .validate_and_publish_message(test_message.clone())
        .unwrap();

    // We expect that the message goes through the forwarder and receiver1
    // even though the forwarder is connected to the receiver2 in the new session.
    loop {
        select! {
            _ = sender.select_next_some() => {}
            event = forwarder.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::Message(message, conn)) = event {
                    assert_eq!(message.id(), test_message.id());
                    forwarder.behaviour_mut()
                        .forward_validated_message(&message, conn)
                        .unwrap();
                }
            }
            event = receiver1.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::Message(message, _)) = event {
                    assert_eq!(message.id(), test_message.id());
                    break;
                }
            }
            _ = receiver2.select_next_some() => {}
        }
    }

    // Now we start the new session for the sender as well.
    // Also, connect the sender to the forwarder for the new session.
    sender
        .behaviour_mut()
        .start_new_session(memberships[0].clone());
    sender
        .connect_and_wait_for_outbound_upgrade(&mut forwarder)
        .await;

    // The sender publishes the same message to the forwarder.
    // It should work because the exchanged message IDs were cleared.
    sender
        .behaviour_mut()
        .validate_and_publish_message(test_message.clone())
        .unwrap();

    // We expect that the message goes through the forwarder and receiver2.
    loop {
        select! {
            _ = sender.select_next_some() => {}
            event = forwarder.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::Message(message, conn)) = event {
                    assert_eq!(message.id(), test_message.id());
                    forwarder.behaviour_mut()
                        .forward_validated_message(&message, conn)
                        .unwrap();
                }
            }
            _ = receiver1.select_next_some() => {}
            event = receiver2.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::Message(message, _)) = event {
                    assert_eq!(message.id(), test_message.id());
                    break;
                }
            }
        }
    }
}

#[test(tokio::test)]
async fn finish_session_transition() {
    let mut dialer = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());
    let mut listener = TestSwarm::new(|id| BehaviourBuilder::default().with_identity(id).build());

    listener.listen().with_memory_addr_external().await;
    dialer
        .connect_and_wait_for_outbound_upgrade(&mut listener)
        .await;

    // Start a new session.
    let memberships = build_memberships(&[&dialer, &listener]);
    dialer
        .behaviour_mut()
        .start_new_session(memberships[0].clone());
    listener
        .behaviour_mut()
        .start_new_session(memberships[1].clone());

    // Finish the transition period
    dialer.behaviour_mut().finish_session_transition();
    listener.behaviour_mut().finish_session_transition();

    // Expect that the connection is closed after 10s (default swarm timeout).
    loop {
        select! {
            _ = dialer.select_next_some() => {}
            event = listener.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { .. } = event {
                    break;
                }
            }
        }
    }
}
