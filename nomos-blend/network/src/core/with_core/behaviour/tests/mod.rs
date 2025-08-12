use futures::StreamExt as _;
use libp2p_swarm_test::SwarmExt as _;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::select;

use crate::{
    core::with_core::behaviour::{
        tests::utils::{TestEncapsulatedMessage, TestSwarm},
        Behaviour, Event,
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
    dialing_swarm.connect(&mut listening_swarm).await;

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

    // Send one message, which is within the range of expected messages.
    let test_message = TestEncapsulatedMessage::new(b"msg");
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
}
