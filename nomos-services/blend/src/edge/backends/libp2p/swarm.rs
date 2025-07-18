use std::time::Duration;

use futures::Stream;
use libp2p::{identity::Keypair, PeerId, Swarm, SwarmBuilder};
use nomos_blend_network::edge::{Behaviour, EventToSwarm};
use nomos_blend_scheduling::membership::Membership;
use nomos_libp2p::{ed25519, SwarmEvent};
use rand::RngCore;
use tokio::sync::mpsc;
use tokio_stream::StreamExt as _;

use super::settings::Libp2pBlendEdgeBackendSettings;
use crate::edge::backends::libp2p::LOG_TARGET;

pub(super) struct BlendEdgeSwarm<SessionStream, Rng>
where
    Rng: RngCore + 'static,
{
    swarm: Swarm<Behaviour<Rng>>,
    command_receiver: mpsc::Receiver<Command>,
    session_stream: SessionStream,
}

#[derive(Debug)]
pub enum Command {
    SendMessage(Vec<u8>),
}

impl<SessionStream, Rng> BlendEdgeSwarm<SessionStream, Rng>
where
    Rng: RngCore + 'static,
{
    pub(super) fn new(
        settings: &Libp2pBlendEdgeBackendSettings,
        session_stream: SessionStream,
        rng: Rng,
        command_receiver: mpsc::Receiver<Command>,
    ) -> Self {
        let keypair = Keypair::from(ed25519::Keypair::from(settings.node_key.clone()));
        let swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_behaviour(|_| Behaviour::new(None, rng))
            .expect("edge::Behaviour should be built")
            .with_swarm_config(|cfg| {
                // The idle timeout starts ticking once there are no active streams on a
                // connection. We want the connection to be closed as soon as
                // all streams are dropped.
                cfg.with_idle_connection_timeout(Duration::ZERO)
            })
            .build();
        Self {
            swarm,
            command_receiver,
            session_stream,
        }
    }

    fn handle_swarm_event(event: SwarmEvent<EventToSwarm>) {
        match event {
            SwarmEvent::Behaviour(EventToSwarm::MessageSuccess(_)) => {
                tracing::debug!(target: LOG_TARGET, "Message has been sent successfully");
            }
            event => {
                tracing::trace!(target: LOG_TARGET, "Unhandled swarm event: {event:?}");
            }
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::SendMessage(msg) => {
                self.handle_send_message_command(msg);
            }
        }
    }

    fn handle_send_message_command(&mut self, msg: Vec<u8>) {
        if let Err(e) = self.swarm.behaviour_mut().send_message(msg) {
            tracing::error!(target: LOG_TARGET, "Failed to schedule sending message: {e}");
        }
    }

    // TODO: Implement the actual session transition.
    //       https://github.com/logos-co/nomos/issues/1462
    fn transition_session(&mut self, membership: Membership<PeerId>) {
        self.swarm.behaviour_mut().set_membership(membership);
    }
}

impl<SessionStream, Rng> BlendEdgeSwarm<SessionStream, Rng>
where
    Rng: RngCore + 'static,
    SessionStream: Stream<Item = Membership<PeerId>> + Unpin,
{
    pub(super) async fn run(mut self) {
        loop {
            tokio::select! {
                Some(event) = self.swarm.next() => {
                    Self::handle_swarm_event(event);
                }
                Some(command) = self.command_receiver.recv() => {
                    self.handle_command(command);
                }
                Some(new_session) = self.session_stream.next() => {
                    self.transition_session(new_session);
                }
            }
        }
    }
}
