use std::time::Duration;

use futures::StreamExt as _;
use libp2p::{Multiaddr, PeerId, Swarm, SwarmBuilder};
use libp2p_stream::OpenStreamError;
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

    let mut timeout = Box::pin(sleep(Duration::from_secs(4)));

    successful_swarm
        .dial(listening_swarm.listeners().next().unwrap().clone())
        .unwrap();
    unsuccessful_swarm
        .dial(listening_swarm.listeners().next().unwrap().clone())
        .unwrap();

    loop {
        tokio::select! {
            () = &mut timeout => {
                break;
            }
            // Consume listening swarm events.
            event = listening_swarm.next() => {
                if let Some(SwarmEvent::Behaviour(Event::Message(msg))) = event {
                    assert_eq!(msg, b"test".to_vec());
                }
            }
            event = successful_swarm.next() => {
                if let Some(SwarmEvent::ConnectionEstablished { peer_id, .. }) = event {
                    let mut successful_swarm_control = successful_swarm.behaviour().new_control();
                    // Upgrade substream to the Blend protocol. This should be successful.
                    tokio::spawn(async move {
                        let stream = successful_swarm_control.open_stream(peer_id, PROTOCOL_NAME).await.expect("Blend stream to be opened successfully.");
                        send_msg(stream, b"test".to_vec()).await.unwrap();
                    });
                }
            }
            event = unsuccessful_swarm.next() => {
                if let Some(SwarmEvent::ConnectionEstablished { peer_id, .. }) = event {
                    let mut unsuccessful_swarm_control = unsuccessful_swarm.behaviour().new_control();
                    // Upgrade substream to the Blend protocol. This should not succeed since behaviour should return a dummy handler because the maximum number of incoming connections has been reached.
                    tokio::spawn(async move {
                        let stream_res = unsuccessful_swarm_control.open_stream(peer_id, PROTOCOL_NAME).await;
                        if let Err(OpenStreamError::UnsupportedProtocol(protocol)) = stream_res {
                            assert_eq!(protocol, PROTOCOL_NAME.as_ref());
                        } else {
                            panic!("Expected UnsupportedProtocol error.");
                        }
                    });
                }
            }
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
            event = listening_swarm.next() => {
                match event {
                    Some(SwarmEvent::Behaviour(Event::Message(msg))) => {
                        assert_eq!(msg, b"test".to_vec());
                    },
                    Some(SwarmEvent::ConnectionClosed { .. }) => {
                        break;
                    }
                    _ => {}
                }
            }
            event = first_swarm.next() => {
                if let Some(SwarmEvent::ConnectionEstablished { peer_id, .. }) = event {
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
