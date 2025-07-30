use std::{process::exit, time::Duration};

use futures::StreamExt as _;
use libp2p::{Multiaddr, PeerId, Swarm, SwarmBuilder};
use nomos_blend_message::crypto::Ed25519PrivateKey;
use nomos_blend_scheduling::membership::{Membership, Node};
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::time::Instant;

use crate::core::with_edge::behaviour::{Behaviour, Config};

#[test(tokio::test)]
async fn incoming_connection_limit_enforced() {
    let mut listening_swarm = core_to_edge_swarm(Membership::new(
        &[Node {
            address: Multiaddr::empty(),
            id: PeerId::random(),
            public_key: Ed25519PrivateKey::generate().public_key(),
        }],
        None,
    ))
    .await;
    let mut dialing_swarm = edge_to_core_swarm();
    let mut dialing_swarm_2 = edge_to_core_swarm();

    dialing_swarm.dial(listening_swarm.listeners().next().unwrap().clone());
    dialing_swarm_2.dial(listening_swarm.listeners().next().unwrap().clone());

    let mut timeout = Box::pin(tokio::time::sleep_until(
        Instant::now() + Duration::from_secs(3),
    ));

    loop {
        tokio::select! {
            () = &mut timeout => {
                exit(0);
            }
            event = listening_swarm.next() => {
                // println!("Event: {event:?}");
            }
            _ = dialing_swarm.next() => {}
            _ = dialing_swarm_2.next() => {}
        }
    }
}

async fn core_to_edge_swarm(core_nodes: Membership<PeerId>) -> Swarm<Behaviour> {
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
