use libp2p::{Multiaddr, PeerId};
use nomos_libp2p::{Protocol, SwarmEvent};
use test_log::test;

use crate::core::backends::libp2p::tests::utils::{BlendBehaviourBuilder, SwarmBuilder, TestSwarm};

#[test(tokio::test)]
async fn redial_same_peer() {
    let TestSwarm {
        swarm: mut dialing_swarm,
        ..
    } = SwarmBuilder::default().build(|id| BlendBehaviourBuilder::new(&id).build());

    let random_peer_id = PeerId::random();
    let empty_multiaddr: Multiaddr = Protocol::Memory(0).into();
    dialing_swarm.dial_peer_at_addr(random_peer_id, empty_multiaddr.clone());

    let dial_attempt_1_record = dialing_swarm.ongoing_dials().get(&random_peer_id).unwrap();
    assert_eq!(*dial_attempt_1_record.address(), empty_multiaddr.clone());
    assert_eq!(
        dial_attempt_1_record.attempt_number(),
        1.try_into().unwrap()
    );

    // We poll the swarm until we know the first dial attempt has failed.
    dialing_swarm
        .poll_swarm_until(|event| {
            let SwarmEvent::OutgoingConnectionError { peer_id, .. } = event else {
                return false;
            };
            *peer_id == Some(random_peer_id)
        })
        .await;

    let dial_attempt_2_record = dialing_swarm.ongoing_dials().get(&random_peer_id).unwrap();
    assert_eq!(*dial_attempt_2_record.address(), empty_multiaddr.clone());
    assert_eq!(
        dial_attempt_2_record.attempt_number(),
        2.try_into().unwrap()
    );

    // We poll the swarm until the next failure.
    dialing_swarm
        .poll_swarm_until(|event| {
            let SwarmEvent::OutgoingConnectionError { peer_id, .. } = event else {
                return false;
            };
            *peer_id == Some(random_peer_id)
        })
        .await;

    let dial_attempt_3_record = dialing_swarm.ongoing_dials().get(&random_peer_id).unwrap();
    assert_eq!(*dial_attempt_3_record.address(), empty_multiaddr.clone());
    assert_eq!(
        dial_attempt_3_record.attempt_number(),
        3.try_into().unwrap()
    );

    // We poll the swarm until the next failure, after which there should be no more
    // attempts.
    dialing_swarm
        .poll_swarm_until(|event| {
            let SwarmEvent::OutgoingConnectionError { peer_id, .. } = event else {
                return false;
            };
            *peer_id == Some(random_peer_id)
        })
        .await;

    // Storage map should be cleared up.
    assert!(dialing_swarm.ongoing_dials().is_empty());
}

#[test(tokio::test)]
async fn redial_different_peer_after_redial_limit() {
    unimplemented!()
}
