use futures::StreamExt as _;
use libp2p::swarm::{dummy, ConnectionError};
use libp2p_swarm_test::SwarmExt as _;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::select;

use crate::core::with_core::behaviour::{tests::utils::TestSwarm, Behaviour};

#[test(tokio::test)]
async fn dialing_peer_not_supporting_blend_protocol() {
    let mut blend_swarm = TestSwarm::new(Behaviour::default());
    let mut dummy_swarm = TestSwarm::new(dummy::Behaviour);

    blend_swarm.listen().with_memory_addr_external().await;
    dummy_swarm.connect(&mut blend_swarm).await;

    let mut events_to_match = 2u8;
    loop {
        select! {
            blend_event = blend_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { peer_id, endpoint, cause, .. } = blend_event {
                    assert_eq!(peer_id, *dummy_swarm.local_peer_id());
                    assert!(endpoint.is_listener());
                    assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                    events_to_match -= 1;
                }
            }
            dummy_event = dummy_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { peer_id, endpoint, cause, .. } = dummy_event {
                    assert_eq!(peer_id, *blend_swarm.local_peer_id());
                    assert!(endpoint.is_dialer());
                    assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
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
async fn listening_peer_not_supporting_blend_protocol() {
    let mut blend_swarm = TestSwarm::new(Behaviour::default());
    let mut dummy_swarm = TestSwarm::new(dummy::Behaviour);

    dummy_swarm.listen().with_memory_addr_external().await;
    blend_swarm.connect(&mut dummy_swarm).await;

    let mut events_to_match = 2u8;
    loop {
        select! {
            blend_event = blend_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { peer_id, endpoint, cause, .. } = blend_event {
                    assert_eq!(peer_id, *dummy_swarm.local_peer_id());
                    assert!(endpoint.is_dialer());
                    assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                    events_to_match -= 1;
                }
            }
            dummy_event = dummy_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { peer_id, endpoint, cause, .. } = dummy_event {
                    assert_eq!(peer_id, *blend_swarm.local_peer_id());
                    assert!(endpoint.is_listener());
                    assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                    events_to_match -= 1;
                }
            }
        }
        if events_to_match == 0 {
            break;
        }
    }
}
