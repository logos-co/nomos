use core::time::Duration;

use libp2p::{core::Endpoint, identity::Keypair};
use libp2p_swarm_test::SwarmExt as _;
use nomos_blend_scheduling::membership::Node;
use nomos_libp2p::SwarmEvent;
use test_log::test;
use tokio::{select, time::sleep};

use crate::core::backends::libp2p::tests::utils::{
    BlendBehaviourBuilder, SwarmBuilder, SwarmExt as _, TestSwarm,
};

#[test(tokio::test)]
async fn redial_same_peer() {
    let listening_swarm_keypair = Keypair::generate_ed25519();
    let TestSwarm {
        swarm: mut listening_swarm,
        ..
    } = SwarmBuilder::default()
        .with_keypair(listening_swarm_keypair.clone())
        .build(|id| BlendBehaviourBuilder::new(&id).build());
    let (
        Node {
            address: listening_swarm_address,
            id: listening_swarm_peer_id,
            ..
        },
        listening_id,
    ) = listening_swarm
        .listen_and_return_membership_entry(None)
        .await;

    // Build dialing swarm with the listening info of the listening swarm.
    let TestSwarm {
        swarm: mut dialing_swarm,
        ..
    } = SwarmBuilder::default().build(|id| BlendBehaviourBuilder::new(&id).build());

    // We stop listening for the listening swarm so we simulate dial failures.
    assert!(listening_swarm.remove_listener_and_wait(listening_id).await);

    dialing_swarm.dial_peer_at_addr(listening_swarm_peer_id, listening_swarm_address.clone());

    // We poll the swarm until we know the first dial attempt has failed.
    dialing_swarm
        .poll_swarm_until(|event| {
            let SwarmEvent::OutgoingConnectionError { peer_id, .. } = event else {
                return false;
            };
            *peer_id == Some(listening_swarm_peer_id)
        })
        .await;

    let dial_attempt_record = dialing_swarm
        .ongoing_dials()
        .get(&listening_swarm_peer_id)
        .unwrap();
    assert_eq!(
        *dial_attempt_record.address(),
        listening_swarm_address.clone(),
    );
    assert_eq!(dial_attempt_record.attempt_number(), 2.try_into().unwrap());

    // Resume listening for the listening swarm.
    listening_swarm
        .listen_and_return_membership_entry(Some(listening_swarm_address.clone()))
        .await;

    loop {
        select! {
            () = sleep(Duration::from_secs(5)) => {
                break;
            }
            () = dialing_swarm.poll_swarm() => {}
            () = listening_swarm.poll_swarm() => {}
        }
    }

    assert!(dialing_swarm.ongoing_dials().is_empty());
    let negotiated_connection_details = dialing_swarm
        .behaviour()
        .blend
        .with_core()
        .negotiated_peers()
        .get(&listening_swarm_peer_id)
        .unwrap();
    assert_eq!(negotiated_connection_details.role(), Endpoint::Listener);
}

#[test(tokio::test)]
async fn redial_different_peer_after_redial_limit() {
    unimplemented!()
}

#[test(tokio::test)]
async fn dial_failure_with_no_more_peers() {
    unimplemented!()
}
