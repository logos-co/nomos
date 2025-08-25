use core::time::Duration;

use libp2p_swarm_test::SwarmExt as _;
use test_log::test;
use tokio::{spawn, time::sleep};

use crate::core::backends::libp2p::{
    swarm::BlendSwarmMessage,
    tests::utils::{BlendBehaviourBuilder, SwarmBuilder, TestEncapsulatedMessage, TestSwarm},
};

#[test(tokio::test)]
async fn message_propagation() {
    let TestSwarm {
        swarm: mut swarm_1,
        swarm_message_sender: swarm_1_message_sender,
        ..
    } = SwarmBuilder::default().build(|id| BlendBehaviourBuilder::new(&id).build());
    let TestSwarm {
        swarm: mut swarm_2, ..
    } = SwarmBuilder::default().build(|id| BlendBehaviourBuilder::new(&id).build());
    let TestSwarm {
        swarm: mut swarm_3,
        incoming_message_receiver: mut swarm_3_message_receiver,
        ..
    } = SwarmBuilder::default().build(|id| BlendBehaviourBuilder::new(&id).build());

    let (swarm_2_address, _) = swarm_2.listen().await;
    let (swarm_3_address, _) = swarm_3.listen().await;

    swarm_1.dial_peer_at_addr(*swarm_2.local_peer_id(), swarm_2_address);
    swarm_2.dial_peer_at_addr(*swarm_3.local_peer_id(), swarm_3_address);

    let message = TestEncapsulatedMessage::new(b"test-payload");

    spawn(async move { swarm_1.run().await });
    spawn(async move { swarm_2.run().await });
    spawn(async move { swarm_3.run().await });

    // Wait for peers to establish connections with each other
    sleep(Duration::from_secs(1)).await;

    swarm_1_message_sender
        .send(BlendSwarmMessage::Publish(message.clone()))
        .await
        .unwrap();

    // We test that swarm 1 publishes a message, sending it to swarm 2, the only
    // swarm it is connected to. Then swarm 2 forwards it to swarm 3, which is not
    // connected to swarm 1.
    let swarm_3_received_message = swarm_3_message_receiver.recv().await.unwrap();
    assert_eq!(swarm_3_received_message.into_inner(), message.into_inner());
}

#[test(tokio::test)]
async fn message_forwarding() {
    unimplemented!()
}

#[test(tokio::test)]
async fn invalid_message_send_attempt() {
    unimplemented!()
}
