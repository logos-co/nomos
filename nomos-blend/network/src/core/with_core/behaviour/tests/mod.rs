use std::collections::HashSet;

use futures::StreamExt as _;
use libp2p_swarm_test::SwarmExt as _;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::select;

use crate::{
    core::with_core::{
        behaviour::{
            tests::utils::{SwarmExt as _, TestEncapsulatedMessage, TestSwarm},
            Behaviour, Event,
        },
        error::Error,
    },
    message::ValidateMessagePublicHeader as _,
};

mod bootstrapping;
mod connection_monitoring;
mod utils;

#[test(tokio::test)]
async fn message_sending_and_reception() {
    let mut dialing_swarm = TestSwarm::new(Behaviour::default());
    let mut listening_swarm = TestSwarm::new(Behaviour::default());

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
async fn invalid_message_publish() {
    let mut dialing_swarm = TestSwarm::new(Behaviour::default());

    let invalid_signature_message = TestEncapsulatedMessage::new_with_invalid_signature(b"data");
    assert_eq!(
        dialing_swarm
            .behaviour_mut()
            .validate_and_publish_message(invalid_signature_message.into_inner()),
        Err(Error::InvalidMessage)
    );
}
