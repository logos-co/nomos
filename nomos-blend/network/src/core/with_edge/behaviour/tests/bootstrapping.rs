use futures::StreamExt as _;
use libp2p::swarm::dummy;
use libp2p_stream::{Behaviour as StreamBehaviour, OpenStreamError};
use libp2p_swarm_test::SwarmExt as _;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::select;

use crate::{
    core::{
        tests::utils::TestSwarm,
        with_edge::behaviour::{Behaviour, Event},
    },
    PROTOCOL_NAME,
};

#[test(tokio::test)]
async fn edge_peer_not_supporting_blend() {
    let mut dummy_swarm = TestSwarm::new(|_| dummy::Behaviour);
    let mut blend_swarm =
        TestSwarm::new(|_| Behaviour::with_edge_peer(*dummy_swarm.local_peer_id()));

    let (blend_address, _) = blend_swarm.listen().await;
    dummy_swarm.dial(blend_address).unwrap();

    let mut events_to_match = 2u8;
    loop {
        select! {
            core_event = blend_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { peer_id, endpoint, .. } = core_event {
                    assert_eq!(peer_id, *dummy_swarm.local_peer_id());
                    assert!(endpoint.is_listener());
                    events_to_match -= 1;
                }
            }
            dummy_event = dummy_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { peer_id, endpoint, .. } = dummy_event {
                    assert_eq!(peer_id, *blend_swarm.local_peer_id());
                    assert!(endpoint.is_dialer());
                    events_to_match -= 1;
                }
            }
        }
        if events_to_match == 0 {
            break;
        }
    }
}

#[test(tokio::test)]
async fn non_edge_peer() {
    let mut other_core_swarm = TestSwarm::new(|_| StreamBehaviour::new());
    let mut blend_swarm =
        TestSwarm::new(|_| Behaviour::with_edge_peer(*other_core_swarm.local_peer_id()));

    let (blend_address, _) = blend_swarm.listen().await;
    other_core_swarm.dial(blend_address).unwrap();

    let mut events_to_match = 3u8;
    loop {
        select! {
            core_event = blend_swarm.select_next_some() => {
                match core_event {
                    SwarmEvent::ConnectionClosed { peer_id, endpoint, .. } => {
                        assert_eq!(peer_id, *other_core_swarm.local_peer_id());
                        assert!(endpoint.is_listener());
                        events_to_match -= 1;
                    }
                    SwarmEvent::Behaviour(Event::Message(_)) => {
                        panic!("Core->edge behaviour should not be able to receive messages from non-edge nodes.");
                    }
                    _ => {}
                }
            }
            other_core_swarm_event = other_core_swarm.select_next_some() => {
                match other_core_swarm_event {
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        assert_eq!(peer_id, *blend_swarm.local_peer_id());
                        // Upgrading the protocol should fail since the core node returns a dummy handler for non-edge peers.
                        let stream_control_res = other_core_swarm.behaviour_mut().new_control().open_stream(*blend_swarm.local_peer_id(), PROTOCOL_NAME).await;
                        assert!(matches!(stream_control_res, Err(OpenStreamError::UnsupportedProtocol(_))));
                        events_to_match -= 1;
                    }
                    SwarmEvent::ConnectionClosed { peer_id, endpoint, .. } => {
                        assert_eq!(peer_id, *blend_swarm.local_peer_id());
                        assert!(endpoint.is_dialer());
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
async fn incoming_connection_with_maximum_peering_degree() {}

#[test(tokio::test)]
async fn concurrent_incoming_connection_and_maximum_peering_degree_reached() {}

#[test(tokio::test)]
async fn outgoing_connection_to_edge_peer() {}
