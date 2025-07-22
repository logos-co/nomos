use core::{
    task::{Context, Poll},
    time::Duration,
};

use futures::{task::noop_waker_ref, StreamExt as _};
use libp2p::swarm::{dial_opts::DialOpts, ConnectionError, SwarmEvent};
use nomos_blend_scheduling::membership::Membership;
use tokio::time::sleep;

use crate::{
    edge::EventToSwarm,
    tests::{core_edge::core_receiver_swarm, edge_core::edge_sender_swarm},
};

mod core_edge;
mod edge_core;

#[test_log::test(tokio::test)]
async fn core_to_edge_connection_failure() {
    let (mut edge_node, edge_node_address) = edge_sender_swarm(None).await;
    let (mut core_node, _) = core_receiver_swarm(Duration::from_secs(1)).await;
    let edge_node_peer_id = *edge_node.local_peer_id();

    core_node
        .dial(DialOpts::from(edge_node_address))
        .expect("Failed to connect to edge node.");

    let mut core_loop_done = false;
    let mut edge_loop_done = false;
    let mut cx = Context::from_waker(noop_waker_ref());
    async {
        loop {
            if !core_loop_done {
                let core_node_event = core_node.poll_next_unpin(&mut cx);
                if let Poll::Ready(Some(SwarmEvent::OutgoingConnectionError { peer_id, .. })) =
                    core_node_event
                {
                    if peer_id == Some(edge_node_peer_id) {
                        core_loop_done = true;
                    }
                }
            }

            if !edge_loop_done {
                let edge_node_event = edge_node.poll_next_unpin(&mut cx);
                if let Poll::Ready(Some(SwarmEvent::ConnectionClosed {
                    cause: Some(ConnectionError::KeepAliveTimeout),
                    ..
                })) = edge_node_event
                {
                    edge_loop_done = true;
                }
            }

            if core_loop_done && edge_loop_done {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
    }
    .await;
}

#[test_log::test(tokio::test)]
async fn message_sending() {
    let (mut core_node, core_node_info) = core_receiver_swarm(Duration::from_secs(1)).await;
    let membership = Membership::new(&[core_node_info], None);
    let (mut edge_node, _) = edge_sender_swarm(Some(membership)).await;

    edge_node
        .behaviour_mut()
        .send_message(b"test".to_vec())
        .expect("Message must be scheduled");

    let mut core_loop_done = false;
    let mut edge_loop_done = false;
    let mut cx = Context::from_waker(noop_waker_ref());
    async {
        loop {
            if !core_loop_done {
                let core_node_event = core_node.poll_next_unpin(&mut cx);
                if let Poll::Ready(Some(SwarmEvent::Behaviour(
                    crate::core::handler::core_edge::ToBehaviour::Message(message),
                ))) = core_node_event
                {
                    if message == b"test".to_vec() {
                        core_loop_done = true;
                    }
                }
            }

            if !edge_loop_done {
                let edge_node_event = edge_node.poll_next_unpin(&mut cx);
                if let Poll::Ready(Some(SwarmEvent::Behaviour(EventToSwarm::MessageSuccess(
                    message,
                )))) = edge_node_event
                {
                    if message == b"test".to_vec() {
                        edge_loop_done = true;
                    }
                }
            }

            if core_loop_done && edge_loop_done {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
    }
    .await;
}

#[test_log::test(tokio::test)]
async fn sender_timeout() {
    // Set timeout to 0 to trigger an immediate timeout.
    let (mut core_node, core_node_info) = core_receiver_swarm(Duration::ZERO).await;
    let membership = Membership::new(&[core_node_info], None);
    let (mut edge_node, _) = edge_sender_swarm(Some(membership)).await;
    let core_node_peer_id = *core_node.local_peer_id();
    let edge_node_peer_id = *edge_node.local_peer_id();

    // Schedule a message to trigger a dialing.
    edge_node.behaviour_mut().send_message(vec![]).unwrap();

    let mut core_loop_done = false;
    let mut edge_loop_done = false;
    let mut cx = Context::from_waker(noop_waker_ref());
    async {
        loop {
            if !core_loop_done {
                let core_node_event = core_node.poll_next_unpin(&mut cx);
                if let Poll::Ready(Some(SwarmEvent::ConnectionEstablished { peer_id, .. })) =
                    core_node_event
                {
                    if peer_id == edge_node_peer_id {
                        core_loop_done = true;
                    }
                }
            }

            // Stop polling once the connection is established,
            // which means that a connection handler has been created
            // and the message will be sent once the connection is fully negotiated.
            // Anyway, we expect that the core node gets a timeout error soon.
            if !edge_loop_done {
                let edge_node_event = edge_node.poll_next_unpin(&mut cx);
                if let Poll::Ready(Some(SwarmEvent::ConnectionEstablished { peer_id, .. })) =
                    edge_node_event
                {
                    if peer_id == core_node_peer_id {
                        edge_loop_done = true;
                    }
                }
            }

            if core_loop_done && edge_loop_done {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
    }
    .await;

    // The next event we should get from the core swarm is a failure due to
    // timeout.
    let event = core_node.next().await;
    let Some(SwarmEvent::Behaviour(crate::core::handler::core_edge::ToBehaviour::FailedReception(
        crate::core::handler::core_edge::FailureReason::Timeout,
    ))) = event
    else {
        panic!("Returned different error than expected (timeout): {event:?}");
    };
}
