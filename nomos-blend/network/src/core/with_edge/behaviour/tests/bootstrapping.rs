use core::time::Duration;

use futures::{AsyncWriteExt as _, StreamExt as _};
use libp2p::{swarm::dummy, PeerId};
use libp2p_stream::{Behaviour as StreamBehaviour, OpenStreamError};
use libp2p_swarm_test::SwarmExt as _;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::{select, spawn, time::sleep};

use crate::{
    core::{
        tests::utils::TestSwarm,
        with_edge::behaviour::{
            tests::utils::{BehaviourBuilder, StreamBehaviourExt as _},
            Event,
        },
    },
    PROTOCOL_NAME,
};

#[test(tokio::test)]
async fn edge_peer_not_supporting_blend() {
    let mut dummy_swarm = TestSwarm::new(|_| dummy::Behaviour);
    let mut blend_swarm = TestSwarm::new(|_| {
        BehaviourBuilder::default()
            // Add random peer to membership so the dummy swarm is considered an edge node.
            .with_edge_peer_membership(PeerId::random())
            .build()
    });

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
    let mut blend_swarm = TestSwarm::new(|_| {
        BehaviourBuilder::default()
            .with_edge_peer_membership(*other_core_swarm.local_peer_id())
            .build()
    });

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
async fn incoming_connection_with_maximum_peering_degree() {
    let mut edge_swarm_1 = TestSwarm::new(|_| StreamBehaviour::new());
    let mut edge_swarm_2 = TestSwarm::new(|_| StreamBehaviour::new());
    let mut blend_swarm = TestSwarm::new(|_| {
        BehaviourBuilder::default()
            // Add random peer to membership so the two swarms are considered edge nodes.
            .with_edge_peer_membership(PeerId::random())
            // We increase the timeout to send a message so that the swarm will close the rejected
            // connection before the behaviour closes the second connection due to inactivity.
            .with_timeout(Duration::from_secs(13))
            .with_max_incoming_connections(1)
            .build()
    });

    let (blend_swarm_address, _) = blend_swarm.listen().with_memory_addr_external().await;

    // We wait that the first connection is established and upgraded.
    edge_swarm_1
        .connect_and_upgrade_to_blend(&mut blend_swarm)
        .await;

    // Then we perform the second dial.
    edge_swarm_2.dial(blend_swarm_address).unwrap();

    let mut edge_swarm_connection_2_closed = false;
    // We verify that the additional connection is closed, and that the old one is
    // kept alive (since we set a timeout of 13 seconds while the default swarm has
    // a timeout of 10 seconds).
    loop {
        select! {
            () = sleep(Duration::from_secs(12)) => {
                break;
            }
             edge_swarm_1_event = edge_swarm_1.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { .. } = edge_swarm_1_event {
                    panic!("Connection with edge swarm 1 should not be closed for the duration of the test.");
                }
            }
            edge_swarm_2_event = edge_swarm_2.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { peer_id, endpoint, .. } = edge_swarm_2_event {
                    assert_eq!(peer_id, *blend_swarm.local_peer_id());
                    assert!(endpoint.is_dialer());
                    edge_swarm_connection_2_closed = true;
                }
            }
            blend_swarm_event = blend_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { peer_id, endpoint, .. } = blend_swarm_event {
                    assert_eq!(peer_id, *edge_swarm_2.local_peer_id());
                    assert!(endpoint.is_listener());
                }
            }
        }
    }

    assert!(edge_swarm_connection_2_closed);
}

#[test(tokio::test)]
async fn concurrent_incoming_connection_and_maximum_peering_degree_reached() {
    let mut listening_swarm = TestSwarm::new(|_| {
        BehaviourBuilder::default()
            .with_max_incoming_connections(1)
            .with_timeout(Duration::from_secs(13))
            .with_edge_peer_membership(PeerId::random())
            .build()
    });
    let listening_swarm_peer_id = *listening_swarm.local_peer_id();
    let mut dialer_swarm_1 = TestSwarm::new(|_| StreamBehaviour::new());
    let mut dialer_swarm_2 = TestSwarm::new(|_| StreamBehaviour::new());

    let (listening_address, _) = listening_swarm.listen().await;

    // Dial concurrently before we poll the listening swarm.
    dialer_swarm_1.dial(listening_address.clone()).unwrap();
    let dialer_1_control = dialer_swarm_1.behaviour_mut().new_control();
    dialer_swarm_2.dial(listening_address).unwrap();
    let dialer_2_control = dialer_swarm_2.behaviour_mut().new_control();

    let mut dialer_1_dropped = false;
    let mut dialer_2_dropped = false;
    loop {
        select! {
            // We make sure that after 12 seconds one of the two connections is dropped (the swarm used in the tests uses a default timeout of 10s and we increased the keepalive timeout for the tested behaviour to 13s).
            // We cannot prevent both connections from being upgraded, but we can test that one of the two is dropped once the listener realized it is above the maximum peering degree.
            // We do not know which one beforehand because they are started in parallel.
            () = sleep(Duration::from_secs(12)) => {
                break;
            }
            _ = listening_swarm.select_next_some() => {}
            dialer_swarm_1_event = dialer_swarm_1.select_next_some() => {
                match dialer_swarm_1_event {
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        assert_eq!(peer_id, *listening_swarm.local_peer_id());
                        let mut dialer_control = dialer_1_control.clone();
                        spawn(async move {let _ = dialer_control.open_stream(listening_swarm_peer_id, PROTOCOL_NAME).await.unwrap().write(b"").await.unwrap();});
                    }
                    SwarmEvent::ConnectionClosed { endpoint, peer_id, .. } => {
                        assert!(!dialer_2_dropped);
                        assert_eq!(peer_id, *listening_swarm.local_peer_id());
                        assert!(endpoint.is_dialer());
                        dialer_1_dropped = true;
                    }
                    _ => {}
                }
            }
            dialer_swarm_2_event = dialer_swarm_2.select_next_some() => {
                match dialer_swarm_2_event {
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        assert_eq!(peer_id, *listening_swarm.local_peer_id());
                        let mut dialer_control = dialer_2_control.clone();
                        spawn(async move {let _ = dialer_control.open_stream(listening_swarm_peer_id, PROTOCOL_NAME).await.unwrap().write(b"").await.unwrap();});
                    }
                    SwarmEvent::ConnectionClosed { endpoint, peer_id, .. } => {
                        assert!(!dialer_1_dropped);
                        assert_eq!(peer_id, *listening_swarm.local_peer_id());
                        assert!(endpoint.is_dialer());
                        dialer_2_dropped = true;
                    }
                    _ => {}
                }
            }
        }
    }

    assert!(dialer_1_dropped ^ dialer_2_dropped);
}

#[test(tokio::test)]
async fn outgoing_connection_to_edge_peer() {
    let mut core_swarm = TestSwarm::new(|_| {
        BehaviourBuilder::default()
            .with_edge_peer_membership(PeerId::random())
            .build()
    });
    let mut edge_swarm = TestSwarm::new(|_| StreamBehaviour::new());
    let edge_swarm_control = edge_swarm.behaviour_mut().new_control();

    let (edge_swarm_address, _) = edge_swarm.listen().await;
    core_swarm.dial(edge_swarm_address).unwrap();

    let mut core_swarm_ready = false;
    let mut edge_swarm_ready = false;
    loop {
        // We don't know which swarm will get notified first, so we need to check the
        // same condition in both cases.
        select! {
            edge_swarm_event = edge_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { peer_id, .. } = edge_swarm_event {
                    assert_eq!(peer_id, *core_swarm.local_peer_id());
                    assert!(!edge_swarm_ready);
                    if core_swarm_ready {
                        let mut control = edge_swarm_control.clone();
                        let stream_control_res = control.open_stream(*core_swarm.local_peer_id(), PROTOCOL_NAME).await;
                        // Since we use a dummy handler for outgoing connections to edge nodes, the stream upgrade should fail.
                        assert!(matches!(stream_control_res, Err(OpenStreamError::UnsupportedProtocol(_))));
                        break;
                    }
                    edge_swarm_ready = true;
                }
            }
            core_swarm_event = core_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { peer_id, .. } = core_swarm_event {
                    assert_eq!(peer_id, *edge_swarm.local_peer_id());
                    assert!(!core_swarm_ready);
                    if edge_swarm_ready {
                        let mut control = edge_swarm_control.clone();
                        let stream_control_res = control.open_stream(*core_swarm.local_peer_id(), PROTOCOL_NAME).await;
                        // Since we use a dummy handler for outgoing connections to edge nodes, the stream upgrade should fail.
                        assert!(matches!(stream_control_res, Err(OpenStreamError::UnsupportedProtocol(_))));
                        break;
                    }
                    core_swarm_ready = true;
                }
            }
        }
    }
}
