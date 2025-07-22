use futures::StreamExt as _;
use libp2p::{
    swarm::{NetworkBehaviour, SwarmEvent},
    Multiaddr, PeerId, Swarm, SwarmBuilder,
};
use nomos_blend_scheduling::membership::Membership;
use rand::rngs::OsRng;

use crate::edge;

pub async fn edge_sender_swarm(
    membership: Option<Membership<PeerId>>,
) -> (Swarm<edge::Behaviour<OsRng>>, Multiaddr) {
    let mut swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| edge::Behaviour::new(membership, OsRng))
        .unwrap()
        .build();
    let _ = swarm
        .listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap())
        .unwrap();
    let listening_addr = wait_for_listening_address(&mut swarm).await;
    (swarm, listening_addr)
}

async fn wait_for_listening_address<Behaviour>(swarm: &mut Swarm<Behaviour>) -> Multiaddr
where
    Behaviour: NetworkBehaviour,
{
    loop {
        if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
            break address;
        }
    }
}
