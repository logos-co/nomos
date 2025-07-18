use core::{
    task::{Context, Poll},
    time::Duration,
};

use edge_core::no_message_edge_sender_swarm;
use futures::{task::noop_waker_ref, StreamExt as _};
use libp2p::swarm::{dial_opts::DialOpts, ListenError, SwarmEvent};
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
    let (mut edge_node, edge_node_address) = edge_sender_swarm().await;
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
                if let Poll::Ready(Some(SwarmEvent::IncomingConnectionError {
                    error: ListenError::Denied { .. },
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
    let (mut edge_node, _) = edge_sender_swarm().await;
    let (mut core_node, core_node_info) = core_receiver_swarm(Duration::from_secs(1)).await;

    edge_node
        .behaviour_mut()
        .set_membership(Membership::new(&[core_node_info], None));

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
    let (mut edge_node, _) = no_message_edge_sender_swarm().await;
    let (mut core_node, core_node_info) = core_receiver_swarm(Duration::from_secs(1)).await;
    let core_node_peer_id = *core_node.local_peer_id();
    let edge_node_peer_id = *edge_node.local_peer_id();

    edge_node
        .behaviour_mut()
        .set_membership(Membership::new(&[core_node_info], None));

    // Schedule a message to trigger a dialing, but it won't be sent actually.
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
                if let Poll::Ready(Some(SwarmEvent::ConnectionEstablished { peer_id, .. })) =
                    core_node_event
                {
                    if peer_id == edge_node_peer_id {
                        core_loop_done = true;
                    }
                }
            }

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
    let Some(SwarmEvent::Behaviour(crate::core::handler::core_edge::ToBehaviour::FailedReception(
        crate::core::handler::core_edge::FailureReason::Timeout,
    ))) = core_node.next().await
    else {
        panic!("Returned different error than expected (timeout)");
    };
}
