use core::time::Duration;

use futures::StreamExt as _;
use libp2p::swarm::ConnectionError;
use libp2p_swarm_test::SwarmExt as _;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::{select, time::sleep};

use crate::core::with_core::behaviour::{
    tests::utils::{SwarmExt as _, TestEncapsulatedMessage, TestSwarm},
    Behaviour, Event, NegotiatedPeerState, SpamReason,
};

#[test(tokio::test)]
async fn detect_spammy_peer() {
    let mut dialing_swarm = TestSwarm::new(Behaviour::default());
    let mut listening_swarm = TestSwarm::new(Behaviour::default());

    listening_swarm.listen().with_memory_addr_external().await;
    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

    // Send two messages when only one was expected.
    dialing_swarm
        .behaviour_mut()
        .validate_and_publish_message(TestEncapsulatedMessage::new(b"msg1").into_inner())
        .unwrap();
    dialing_swarm
        .behaviour_mut()
        .validate_and_publish_message(TestEncapsulatedMessage::new(b"msg2").into_inner())
        .unwrap();

    let mut events_to_match = 2u8;
    loop {
        select! {
            _ = dialing_swarm.select_next_some() => {}
            listening_event = listening_swarm.select_next_some() => {
                match listening_event {
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, NegotiatedPeerState::Spammy(SpamReason::TooManyMessages))) => {
                        assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                        assert!(listening_swarm.behaviour().negotiated_peers.is_empty());
                        events_to_match -= 1;
                    }
                    SwarmEvent::ConnectionClosed { peer_id, endpoint, cause, .. } => {
                        assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                        assert!(endpoint.is_listener());
                        assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
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
async fn detect_unhealthy_peer() {
    let mut dialing_swarm = TestSwarm::new(Behaviour::default());
    let mut listening_swarm = TestSwarm::new(Behaviour::default());

    listening_swarm.listen().with_memory_addr_external().await;
    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

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
            () = sleep(Duration::from_secs(5)) => {
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
    let mut dialing_swarm = TestSwarm::new(Behaviour::default());
    let mut listening_swarm = TestSwarm::new(Behaviour::default());

    listening_swarm.listen().with_memory_addr_external().await;
    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

    // Let the connection turn unhealthy.
    sleep(Duration::from_secs(4)).await;

    // Send a message to the listening swarm to revert from unhealthy to healthy.
    dialing_swarm
        .behaviour_mut()
        .force_send_message_to_peer(
            &TestEncapsulatedMessage::new(b"msg"),
            *listening_swarm.local_peer_id(),
        )
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

#[test(tokio::test)]
async fn incoming_attempt_with_max_negotiated_peering_degree() {
    let mut listening_swarm = TestSwarm::new(Behaviour::default());
    let mut dialer_swarm_1 = TestSwarm::new(Behaviour::default());
    let mut dialer_swarm_2 = TestSwarm::new(Behaviour::default());

    listening_swarm.listen().with_memory_addr_external().await;

    dialer_swarm_1
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;

    // We can call `connect` since a new connection will be established, but
    // will fail to upgrade (which we test below).
    dialer_swarm_2.connect(&mut listening_swarm).await;

    loop {
        select! {
            listening_swarm_event = listening_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { peer_id, endpoint, cause, .. } = listening_swarm_event {
                    assert_eq!(peer_id, *dialer_swarm_2.local_peer_id());
                    assert!(endpoint.is_listener());
                    assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                    break;
                }
            }
            _ = dialer_swarm_1.select_next_some() => {}
            _ = dialer_swarm_2.select_next_some() => {}
        }
    }
}

#[test(tokio::test)]
async fn concurrent_incoming_connections() {
    let mut listening_swarm = TestSwarm::new(Behaviour::default());
    let mut dialer_swarm_1 = TestSwarm::new(Behaviour::default());
    let mut dialer_swarm_2 = TestSwarm::new(Behaviour::default());

    let (listening_address, _) = listening_swarm.listen().await;

    // Dial concurrently before we poll the listening swarm.
    dialer_swarm_1.dial(listening_address.clone()).unwrap();
    dialer_swarm_2.dial(listening_address).unwrap();

    let mut dialer_1_dropped = false;
    let mut dialer_1_notified = false;
    let mut dialer_2_dropped = false;
    let mut dialer_2_notified = false;
    loop {
        select! {
            // We make sure that after 11 seconds one of the two connections is dropped (the swarm used in the tests uses a default timeout of 10s).
            // We cannot prevent both connections from being upgraded, but we can test that one of the two is dropped once the listener realized it is above the maximum peering degree.
            // We do not know which one beforehand because they are started in parallel.
            () = sleep(Duration::from_secs(11)) => {
                break;
            }
            listening_swarm_event = listening_swarm.select_next_some() => {
                // We check that the listening swarm never generates a `PeerDisconnected` event because it knows the dropped connection is meant to be ignored.
                if let SwarmEvent::Behaviour(Event::PeerDisconnected(_, _)) = listening_swarm_event {
                    panic!("Should not generate a `PeerDisconnected` event for a peer that went above our peering degree.");
                }
            }
            dialer_swarm_1_event = dialer_swarm_1.select_next_some() => {
                match dialer_swarm_1_event {
                    SwarmEvent::ConnectionClosed { cause, endpoint, peer_id, .. } => {
                        assert!(!dialer_2_dropped);
                        assert_eq!(peer_id, *listening_swarm.local_peer_id());
                        assert!(endpoint.is_dialer());
                        assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                        dialer_1_dropped = true;
                    }
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, _)) if peer_id == *listening_swarm.local_peer_id() => {
                        dialer_1_notified = true;
                    }
                    _ => {}
                }
            }
            dialer_swarm_2_event = dialer_swarm_2.select_next_some() => {
                match dialer_swarm_2_event {
                    SwarmEvent::ConnectionClosed { cause, endpoint, peer_id, .. } => {
                        assert!(!dialer_1_dropped);
                        assert_eq!(peer_id, *listening_swarm.local_peer_id());
                        assert!(endpoint.is_dialer());
                        assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                        dialer_2_dropped = true;
                    }
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, _)) if peer_id == *listening_swarm.local_peer_id() => {
                        dialer_2_notified = true;
                    }
                    _ => {}
                }
            }
        }
    }

    // We check whether the dialer whose connection was dropped was also notified by
    // its behaviour that the dialed peer got disconnected.
    assert!((dialer_1_dropped && dialer_1_notified) || (dialer_2_dropped && dialer_2_notified));
}

#[test(tokio::test)]
async fn concurrent_incoming_connections_from_same_peer() {
    unimplemented!()
}

#[test(tokio::test)]
async fn incoming_attempt_with_duplicate_connection() {
    let mut listening_swarm = TestSwarm::new(Behaviour::default());
    let mut dialer_swarm_1 = TestSwarm::new(Behaviour::default());

    listening_swarm.listen().with_memory_addr_external().await;

    dialer_swarm_1
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;
    // This call will result in a closed connection.
    dialer_swarm_1.connect(&mut listening_swarm).await;

    loop {
        select! {
            () = sleep(Duration::from_secs(11)) => {
                break;
            }
            listening_swarm_event = listening_swarm.select_next_some() => {
                match listening_swarm_event {
                    SwarmEvent::ConnectionClosed { peer_id, endpoint, cause, .. } => {
                        assert_eq!(peer_id, *dialer_swarm_1.local_peer_id());
                        assert!(endpoint.is_listener());
                        assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                    }
                    // Listener swarm should not know about this
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, _)) => {
                        assert!(peer_id != *dialer_swarm_1.local_peer_id());
                    }
                    _ => {}
                }
            }
            // Neither should the dialer
            dialer_swarm_event = dialer_swarm_1.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, _)) = dialer_swarm_event {
                    assert!(peer_id != *dialer_swarm_1.local_peer_id());
                }
            }
        }
    }
}

#[test(tokio::test)]
async fn outgoing_attempt_with_max_negotiated_peering_degree() {
    let mut dialing_swarm = TestSwarm::new(Behaviour::default());
    let mut listening_swarm_1 = TestSwarm::new(Behaviour::default());
    let mut listening_swarm_2 = TestSwarm::new(Behaviour::default());

    listening_swarm_1.listen().with_memory_addr_external().await;
    listening_swarm_2.listen().with_memory_addr_external().await;

    dialing_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm_1)
        .await;

    // We can call `connect` since a new connection will be established, but
    // will fail to upgrade (which we test below).
    dialing_swarm.connect(&mut listening_swarm_2).await;

    loop {
        select! {
            dialer_swarm_event = dialing_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionClosed { peer_id, endpoint, cause, .. } = dialer_swarm_event {
                    assert_eq!(peer_id, *listening_swarm_2.local_peer_id());
                    assert!(endpoint.is_dialer());
                    assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                    break;
                }
            }
            _ = listening_swarm_1.select_next_some() => {}
            _ = listening_swarm_2.select_next_some() => {}
        }
    }
}

#[test(tokio::test)]
async fn concurrent_outgoing_connections() {
    let mut dialing_swarm = TestSwarm::new(Behaviour::default());
    let mut listening_swarm_1 = TestSwarm::new(Behaviour::default());
    let mut listening_swarm_2 = TestSwarm::new(Behaviour::default());

    let (listening_address_1, _) = listening_swarm_1.listen().await;
    let (listening_address_2, _) = listening_swarm_2.listen().await;

    dialing_swarm.dial(listening_address_1).unwrap();
    dialing_swarm.dial(listening_address_2).unwrap();

    let mut listener_1_dropped = false;
    let mut listener_1_notified = false;
    let mut listener_2_dropped = false;
    let mut listener_2_notified = false;
    loop {
        select! {
            // We make sure that after 11 seconds one of the two connections is dropped (the swarm used in the tests uses a default timeout of 10s).
            // We cannot prevent both connections from being upgraded, but we can test that one of the two is dropped once the dialer realized it is above the maximum peering degree.
            // We do not know which one beforehand because they are started in parallel.
            () = sleep(Duration::from_secs(11)) => {
                break;
            },
            dialing_swarm_event = dialing_swarm.select_next_some() => {
                // We check that the dialing swarm never generates a `PeerDisconnected` event because it knows the dropped connection is meant to be ignored.
                if let SwarmEvent::Behaviour(Event::PeerDisconnected(_, _)) = dialing_swarm_event {
                    panic!("Should not generate a `PeerDisconnected` event for a peer that went above our peering degree.");
                }
            }
            listener_swarm_1_event = listening_swarm_1.select_next_some() => {
                match listener_swarm_1_event {
                    SwarmEvent::ConnectionClosed { cause, endpoint, peer_id, .. } => {
                        assert!(!listener_2_dropped);
                        assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                        assert!(endpoint.is_listener());
                        assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                        listener_1_dropped = true;
                    }
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, _)) if peer_id == *dialing_swarm.local_peer_id() => {
                        listener_1_notified = true;
                    }
                    _ => {}
                }
            }
            listener_swarm_2_event = listening_swarm_2.select_next_some() => {
                match listener_swarm_2_event {
                    SwarmEvent::ConnectionClosed { cause, endpoint, peer_id, .. } => {
                        assert!(!listener_1_dropped);
                        assert_eq!(peer_id, *dialing_swarm.local_peer_id());
                        assert!(endpoint.is_listener());
                        assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                        listener_2_dropped = true;
                    }
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, _)) if peer_id == *dialing_swarm.local_peer_id() => {
                        listener_2_notified = true;
                    }
                    _ => {}
                }
            }
        }
    }

    // We check whether the listener whose connection was dropped was also notified
    // by its behaviour that the dialed peer got disconnected.
    assert!(
        (listener_1_dropped && listener_1_notified) || (listener_2_dropped && listener_2_notified)
    );
}

#[test(tokio::test)]
async fn concurrent_outgoing_connections_to_same_peer() {
    unimplemented!()
}

#[test(tokio::test)]
async fn outgoing_attempt_with_duplicate_connection() {
    let mut dialer_swarm = TestSwarm::new(Behaviour::default());
    let mut listening_swarm = TestSwarm::new(Behaviour::default());

    listening_swarm.listen().with_memory_addr_external().await;

    dialer_swarm
        .connect_and_wait_for_outbound_upgrade(&mut listening_swarm)
        .await;
    // This call will result in a closed connection.
    dialer_swarm.connect(&mut listening_swarm).await;

    loop {
        select! {
            () = sleep(Duration::from_secs(11)) => {
                break;
            }
             listening_swarm_event = listening_swarm.select_next_some() => {
                match listening_swarm_event {
                    SwarmEvent::ConnectionClosed { peer_id, endpoint, cause, .. } => {
                        assert_eq!(peer_id, *dialer_swarm.local_peer_id());
                        assert!(endpoint.is_listener());
                        assert!(matches!(cause, Some(ConnectionError::KeepAliveTimeout)));
                    }
                    // Listener swarm should not know about this
                    SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, _)) => {
                        assert!(peer_id != *dialer_swarm.local_peer_id());
                    }
                    _ => {}
                }
            }
            // Neither should the dialer
            dialer_swarm_event = dialer_swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, _)) = dialer_swarm_event {
                    assert!(peer_id != *dialer_swarm.local_peer_id());
                }
            }
        }
    }
}

#[test(tokio::test)]
async fn replacing_outgoing_with_incoming_connection() {
    unimplemented!()
}

#[test(tokio::test)]
async fn discarding_outgoing_for_existing_incoming_connection() {
    unimplemented!()
}

#[test(tokio::test)]
async fn replacing_incoming_with_outgoing_connection() {
    unimplemented!()
}

#[test(tokio::test)]
async fn discarding_incoming_for_existing_outgoing_connection() {
    unimplemented!()
}

#[test(tokio::test)]
async fn concurrent_reverse_connections_between_peers() {
    unimplemented!()
}
