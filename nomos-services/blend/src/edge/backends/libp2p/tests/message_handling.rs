use core::{slice::from_ref, time::Duration};

use nomos_blend_scheduling::membership::Membership;
use test_log::test;
use tokio::{select, spawn, time::sleep};

use crate::{
    core::backends::libp2p::core_swarm_test_utils::{
        BlendBehaviourBuilder, SwarmBuilder as CoreSwarmBuilder, SwarmExt as _,
        TestSwarm as CoreTestSwarm,
    },
    edge::backends::libp2p::{
        swarm::Command,
        tests::utils::{SwarmBuilder as EdgeSwarmBuilder, TestSwarm as EdgeTestSwarm},
    },
    test_utils::{TestEncapsulatedMessage, crypto::MockProofsVerifier},
};

#[test(tokio::test)]
async fn edge_message_propagation() {
    let CoreTestSwarm {
        swarm: mut core_swarm_1,
        incoming_message_receiver: mut core_swarm_1_incoming_message_receiver,
        ..
    } = CoreSwarmBuilder::default()
        .build(|id| BlendBehaviourBuilder::new(&id, MockProofsVerifier).build());
    let (swarm_1_membership_entry, _) = core_swarm_1.listen_and_return_membership_entry(None).await;

    let membership = Membership::new_without_local(from_ref(&swarm_1_membership_entry));
    let CoreTestSwarm {
        swarm: mut core_swarm_2,
        incoming_message_receiver: mut core_swarm_2_incoming_message_receiver,
        ..
    } = CoreSwarmBuilder::default()
        .with_membership(membership.clone())
        .build(|id| {
            BlendBehaviourBuilder::new(&id, MockProofsVerifier)
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
    let membership_for_edge_swarm =
        Membership::new_without_local(from_ref(&swarm_2_membership_entry));
    let EdgeTestSwarm {
        swarm: edge_swarm,
        command_sender: edge_swarm_command_sender,
    } = EdgeSwarmBuilder::new(membership_for_edge_swarm)
        // We test that we can pick the `min` between the replication factor and the available
        // peers.
        .with_replication_factor(usize::MAX)
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

#[test(tokio::test)]
async fn replication_factor() {
    let CoreTestSwarm {
        swarm: mut core_swarm_1,
        incoming_message_receiver: mut core_swarm_1_incoming_message_receiver,
        ..
    } = CoreSwarmBuilder::default()
        .with_empty_membership()
        .build(|id| {
            BlendBehaviourBuilder::new(&id, MockProofsVerifier)
                .with_empty_membership()
                .build()
        });
    let (swarm_1_membership_entry, _) = core_swarm_1.listen_and_return_membership_entry(None).await;

    let CoreTestSwarm {
        swarm: mut core_swarm_2,
        incoming_message_receiver: mut core_swarm_2_incoming_message_receiver,
        ..
    } = CoreSwarmBuilder::default()
        .with_empty_membership()
        .build(|id| {
            BlendBehaviourBuilder::new(&id, MockProofsVerifier)
                .with_empty_membership()
                .build()
        });
    let (swarm_2_membership_entry, _) = core_swarm_2.listen_and_return_membership_entry(None).await;

    let CoreTestSwarm {
        swarm: mut core_swarm_3,
        incoming_message_receiver: mut core_swarm_3_incoming_message_receiver,
        ..
    } = CoreSwarmBuilder::default()
        .with_empty_membership()
        .build(|id| {
            BlendBehaviourBuilder::new(&id, MockProofsVerifier)
                .with_empty_membership()
                .build()
        });
    let (swarm_3_membership_entry, _) = core_swarm_3.listen_and_return_membership_entry(None).await;

    spawn(async move { core_swarm_1.run().await });
    spawn(async move { core_swarm_2.run().await });
    spawn(async move { core_swarm_3.run().await });

    // We pass all 3 swarms to the edge swarm, and we test that only 2 of them
    // (replication factor) are picked.
    let membership_for_edge_swarm = Membership::new_without_local(&[
        swarm_1_membership_entry,
        swarm_2_membership_entry,
        swarm_3_membership_entry,
    ]);
    let EdgeTestSwarm {
        swarm: edge_swarm,
        command_sender: edge_swarm_command_sender,
    } = EdgeSwarmBuilder::new(membership_for_edge_swarm)
        .with_replication_factor(2)
        .build();
    spawn(async move { edge_swarm.run().await });

    // Send message
    let message = TestEncapsulatedMessage::new(b"test-payload");
    edge_swarm_command_sender
        .send(Command::SendMessage(message.clone()))
        .await
        .unwrap();

    let mut received_messages = 0u8;
    let mut swarm_1_message_received = false;
    let mut swarm_2_message_received = false;
    let mut swarm_3_message_received = false;
    loop {
        select! {
            () = sleep(Duration::from_secs(5)) => {
                break;
            }
            swarm_1_received_message = core_swarm_1_incoming_message_receiver.recv() => {
                assert_eq!(swarm_1_received_message.unwrap().into_inner(), message.clone());
                assert!(!swarm_1_message_received);
                received_messages += 1;
                swarm_1_message_received = true;
            }
            swarm_2_received_message = core_swarm_2_incoming_message_receiver.recv() => {
                assert_eq!(swarm_2_received_message.unwrap().into_inner(), message.clone());
                assert!(!swarm_2_message_received);
                received_messages += 1;
                swarm_2_message_received = true;
            }
            swarm_3_received_message = core_swarm_3_incoming_message_receiver.recv() => {
                assert_eq!(swarm_3_received_message.unwrap().into_inner(), message.clone());
                assert!(!swarm_3_message_received);
                received_messages += 1;
                swarm_3_message_received = true;
            }
        }
    }

    // Verify that only 2 out of 3 (unconnected) core swarms receive the message.
    assert_eq!(received_messages, 2);
    assert!(!swarm_1_message_received || !swarm_2_message_received || !swarm_3_message_received);
}
