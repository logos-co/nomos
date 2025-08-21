use std::{io, time::Duration};

use futures::StreamExt as _;
use libp2p::{swarm::ConnectionError, PeerId};
use libp2p_swarm_test::SwarmExt;
use nomos_blend_scheduling::serialize_encapsulated_message;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::{select, time::sleep};

use crate::{
    core::{
        tests::utils::{TestEncapsulatedMessage, TestSwarm},
        with_edge::behaviour::{
            tests::utils::{BehaviourBuilder, StreamBehaviourExt},
            Event,
        },
    },
    send_msg,
};

#[test(tokio::test)]
async fn close_remaining_connections() {
    let mut core = TestSwarm::new(|_| {
        BehaviourBuilder::default()
            .with_core_peer_membership(PeerId::random())
            // We increase the inactivity timeout higher than the default swarm timeout 10s,
            // so that we can check if the connection is closed 10s after a new session
            // is started and the core node closes the substream.
            .with_timeout(Duration::from_secs(13))
            .build()
    });
    let mut edge = TestSwarm::new(|_| libp2p_stream::Behaviour::new());

    core.listen().with_memory_addr_external().await;
    let stream = edge.connect_and_upgrade_to_blend(&mut core).await;

    // Wait until the connection is negotiated.
    loop {
        select! {
            _ = edge.select_next_some() => {}
            event = core.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::InboundConnectionUpgradeSucceeded(_)) = event {
                    break;
                }
            }
        }
    }

    // Start a new session without sending any messages.
    // We reuse the same membership since it's not important for this test.
    let membership = core.behaviour().current_membership.clone().unwrap();
    core.behaviour_mut().start_new_session(membership);

    // Expect that the connection is closed after 10s (default swarm timeout).
    loop {
        select! {
            () = sleep(Duration::from_secs(12)) => {
                unreachable!("The connection must be closed earlier");
            }
            _ = edge.select_next_some() => {}
            event = core.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { .. } = event {
                    break;
                }
            }
        }
    }
}
