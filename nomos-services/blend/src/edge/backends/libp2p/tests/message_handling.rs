use core::{slice::from_ref, time::Duration};

use nomos_blend_scheduling::membership::Membership;
use test_log::test;
use tokio::{spawn, time::sleep};

use crate::{
    core::backends::libp2p::core_swarm_test_utils::{
        BlendBehaviourBuilder, SwarmBuilder as CoreSwarmBuilder, SwarmExt as _,
        TestSwarm as CoreTestSwarm,
    },
    edge::backends::libp2p::{
        swarm::Command,
        tests::utils::{SwarmBuilder as EdgeSwarmBuilder, TestSwarm as EdgeTestSwarm},
    },
    test_utils::TestEncapsulatedMessage,
};

#[test(tokio::test)]
async fn edge_message_propagation() {
    let CoreTestSwarm {
        swarm: mut core_swarm_1,
        incoming_message_receiver: mut core_swarm_1_incoming_message_receiver,
        ..
    } = CoreSwarmBuilder::default().build(|id| BlendBehaviourBuilder::new(&id).build());
    let (swarm_1_membership_entry, _) = core_swarm_1.listen_and_return_membership_entry(None).await;

    let membership = Membership::new(from_ref(&swarm_1_membership_entry), None);
    let CoreTestSwarm {
        swarm: mut core_swarm_2,
        incoming_message_receiver: mut core_swarm_2_incoming_message_receiver,
        ..
    } = CoreSwarmBuilder::default()
        .with_membership(membership.clone())
        .build(|id| {
            BlendBehaviourBuilder::new(&id)
                .with_membership(membership)
                .build()
        });
    let (swarm_2_membership_entry, _) = core_swarm_2.listen_and_return_membership_entry(None).await;

    // We connect swarm 2 to swarm 1.
    core_swarm_2.dial_peer_at_addr(
        swarm_1_membership_entry.id,
        swarm_1_membership_entry.address,
    );

    spawn(async move { core_swarm_1.run().await });
    spawn(async move { core_swarm_2.run().await });

    // Wait for peers to establish connections with each other
    sleep(Duration::from_secs(1)).await;

    // We pass swarm 2 to the edge swarm, which will select it to propagate its
    // message.
    let membership_for_edge_swarm = Membership::new(from_ref(&swarm_2_membership_entry), None);
    let EdgeTestSwarm {
        swarm: edge_swarm,
        command_sender: edge_swarm_command_sender,
    } = EdgeSwarmBuilder::default()
        .with_membership(membership_for_edge_swarm)
        .build();
    spawn(async move { edge_swarm.run().await });

    // Send message
    let message = TestEncapsulatedMessage::new(b"test-payload");
    edge_swarm_command_sender
        .send(Command::SendMessage(message.clone()))
        .await
        .unwrap();

    // Verify that both peers receive the message, even though the edge swarm is
    // connected to only one of them.
    let swarm_1_received_message = core_swarm_1_incoming_message_receiver.recv().await.unwrap();
    let swarm_2_received_message = core_swarm_2_incoming_message_receiver.recv().await.unwrap();

    assert_eq!(swarm_1_received_message.into_inner(), message.clone());
    assert_eq!(swarm_2_received_message.into_inner(), message.clone());
}
