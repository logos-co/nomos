use std::time::Duration;

use futures::StreamExt as _;
use libp2p::{Multiaddr, PeerId, Swarm, SwarmBuilder};
use nomos_blend_message::crypto::Ed25519PrivateKey;
use nomos_blend_scheduling::membership::{Membership, Node};
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::time::sleep;

use crate::{
    core::with_edge::behaviour::{Behaviour, Config, Event},
    send_msg, PROTOCOL_NAME,
};

#[test(tokio::test)]
async fn incoming_connection_limit_enforced() {
    let mut listening_swarm = core_to_edge_swarm(
        Membership::new(
            &[Node {
                address: Multiaddr::empty(),
                id: PeerId::random(),
                public_key: Ed25519PrivateKey::generate().public_key(),
            }],
            None,
        ),
        Some(Duration::from_secs(1)),
    )
    .await;
    let mut successful_swarm = edge_to_core_swarm();
    let mut unsuccessful_swarm = edge_to_core_swarm();

    successful_swarm
        .dial(listening_swarm.listeners().next().unwrap().clone())
        .unwrap();
    // Small delay to make sure first dial attempt happens before second.
    sleep(Duration::from_millis(500)).await;
    unsuccessful_swarm
        .dial(listening_swarm.listeners().next().unwrap().clone())
        .unwrap();

    let mut num_events_waiting: u8 = 3;

    loop {
        if num_events_waiting == 0 {
            break;
        }

        tokio::select! {
            event = listening_swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(Event::IncomingConnectionRequestAccepted(peer_id, _)) => {
                        assert_eq!(peer_id, *successful_swarm.local_peer_id());
                        num_events_waiting -= 1;
                    }
                    SwarmEvent::Behaviour(Event::IncomingConnectionRequestDiscarded(peer_id, _)) => {
                        assert_eq!(peer_id, *unsuccessful_swarm.local_peer_id());
                        num_events_waiting -= 1;
                    }
                    SwarmEvent::Behaviour(Event::Message(msg)) => {
                        assert_eq!(msg, b"test".to_vec());
                        num_events_waiting -= 1;
                    }
                    _ => {}
                }
            }
            event = successful_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { peer_id, .. } = event {
                    assert_eq!(peer_id, *listening_swarm.local_peer_id());
                    let mut successful_swarm_control = successful_swarm.behaviour().new_control();
                    // Upgrade substream to the Blend protocol. This should be successful.
                    tokio::spawn(async move {
                        let stream = successful_swarm_control.open_stream(peer_id, PROTOCOL_NAME).await.expect("Blend stream to be opened successfully.");
                        send_msg(stream, b"test".to_vec()).await.unwrap();
                    });
                }
            }
            _ = unsuccessful_swarm.select_next_some() => {}
        }
    }
}

#[test(tokio::test)]
async fn connect_disconnect_connect() {
    let mut listening_swarm = core_to_edge_swarm(
        Membership::new(
            &[Node {
                address: Multiaddr::empty(),
                id: PeerId::random(),
                public_key: Ed25519PrivateKey::generate().public_key(),
            }],
            None,
        ),
        Some(Duration::from_secs(1)),
    )
    .await;
    let mut first_swarm = edge_to_core_swarm();
    let mut second_swarm = edge_to_core_swarm();

    first_swarm
        .dial(listening_swarm.listeners().next().unwrap().clone())
        .unwrap();

    loop {
        tokio::select! {
            // Consume listening swarm events.
            event = listening_swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(Event::Message(msg)) => {
                        assert_eq!(msg, b"test".to_vec());
                    },
                    SwarmEvent::ConnectionClosed { .. } => {
                        break;
                    }
                    _ => {}
                }
            }
            event = first_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { peer_id, .. } = event {
                    let mut successful_swarm_control = first_swarm.behaviour().new_control();
                    // Upgrade substream to the Blend protocol. This should be successful.
                    tokio::spawn(async move {
                        let stream = successful_swarm_control.open_stream(peer_id, PROTOCOL_NAME).await.expect("Blend stream to be opened successfully.");
                        send_msg(stream, b"test".to_vec()).await.unwrap();
                    });
                }
            }
        }
    }

    second_swarm
        .dial(listening_swarm.listeners().next().unwrap().clone())
        .unwrap();

    loop {
        tokio::select! {
            // Consume listening swarm events.
            event = listening_swarm.next() => {
                match event {
                    Some(SwarmEvent::Behaviour(Event::Message(msg))) => {
                        assert_eq!(msg, b"test-2".to_vec());
                    },
                    Some(SwarmEvent::ConnectionClosed { .. }) => {
                        break;
                    }
                    _ => {}
                }
            }
            event = second_swarm.next() => {
                if let Some(SwarmEvent::ConnectionEstablished { peer_id, .. }) = event {
                    let mut successful_swarm_control = second_swarm.behaviour().new_control();
                    tokio::spawn(async move {
                        let stream = successful_swarm_control.open_stream(peer_id, PROTOCOL_NAME).await.expect("Blend stream to be opened successfully.");
                        send_msg(stream, b"test-2".to_vec()).await.unwrap();
                    });
                }
            }
        }
    }
}

async fn core_to_edge_swarm(
    core_nodes: Membership<PeerId>,
    timeout_duration: Option<Duration>,
) -> Swarm<Behaviour> {
    let mut swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| {
            Behaviour::new(
                &Config {
                    connection_timeout: Duration::from_secs(1),
                    max_incoming_connections: 1,
                },
                Some(core_nodes),
            )
        })
        .unwrap()
        .with_swarm_config(|cfg| {
            if let Some(timeout) = timeout_duration {
                return cfg.with_idle_connection_timeout(timeout);
            }
            cfg
        })
        .build();
    let _ = swarm.listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap());
    wait_for_listening_address(&mut swarm).await;
    swarm
}

async fn wait_for_listening_address(swarm: &mut Swarm<Behaviour>) -> Multiaddr {
    loop {
        if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
            break address;
        }
    }
}

fn edge_to_core_swarm() -> Swarm<libp2p_stream::Behaviour> {
    SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| libp2p_stream::Behaviour::new())
        .unwrap()
        .build()
}
