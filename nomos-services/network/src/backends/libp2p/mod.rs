mod command;
mod config;
pub(crate) mod swarm;

use self::swarm::SwarmHandler;
pub use self::{
    command::{Command, Dial, Libp2pInfo, Topic},
    config::Libp2pConfig,
};
use super::NetworkBackend;
use cryptarchia_engine::Slot;
use cryptarchia_sync_network::behaviour::BehaviourSyncEvent::TipRequest;
use cryptarchia_sync_network::behaviour::{
    BehaviourSyncEvent::SyncRequest, BehaviourSyncReply, SyncDirection,
};
use nomos_core::header::HeaderId;
pub use nomos_libp2p::libp2p::gossipsub::{Message, TopicHash};
use nomos_libp2p::{gossipsub, BehaviourEvent};
use overwatch::{overwatch::handle::OverwatchHandle, services::state::NoState};
use tokio::sync::{broadcast, mpsc, mpsc::Sender};

pub struct Libp2p {
    events_tx: broadcast::Sender<Event>,
    commands_tx: Sender<Command>,
}

#[derive(Debug)]
pub enum EventKind {
    Message,
    SyncRequest,
}

#[derive(Debug, Clone)]
pub enum SyncRequestKind {
    ForwardChain(Slot),
    BackwardChain(HeaderId),
    Tip,
}

/// Events emitted from [`NomosLibp2p`], which users can subscribe
#[derive(Debug, Clone)]
pub enum Event {
    Message(Message),
    IncomingSyncRequest {
        kind: SyncRequestKind,
        reply_channel: Sender<BehaviourSyncReply>,
    },
}

impl TryFrom<BehaviourEvent> for Event {
    type Error = Box<dyn std::error::Error>;

    fn try_from(event: BehaviourEvent) -> Result<Self, Self::Error> {
        match event {
            BehaviourEvent::Gossipsub(gossipsub::Event::Message {
                propagation_source: _propagation_source,
                message_id: _message_id,
                message,
            }) => Ok(Self::Message(message)),
            BehaviourEvent::Sync(SyncRequest {
                direction,
                response_sender,
            }) => match direction {
                SyncDirection::Forward(slot) => Ok(Self::IncomingSyncRequest {
                    kind: SyncRequestKind::ForwardChain(slot),
                    reply_channel: response_sender,
                }),
                SyncDirection::Backward(header_id) => Ok(Self::IncomingSyncRequest {
                    kind: SyncRequestKind::BackwardChain(header_id),
                    reply_channel: response_sender,
                }),
            },
            BehaviourEvent::Sync(TipRequest { response_sender }) => Ok(Self::IncomingSyncRequest {
                kind: SyncRequestKind::Tip,
                reply_channel: response_sender,
            }),
            _ => Err("Event not supported".into()),
        }
    }
}

const BUFFER_SIZE: usize = 64;

#[async_trait::async_trait]
impl<RuntimeServiceId> NetworkBackend<RuntimeServiceId> for Libp2p {
    type Settings = Libp2pConfig;
    type State = NoState<Libp2pConfig>;
    type Message = Command;
    type EventKind = EventKind;
    type NetworkEvent = Event;

    fn new(config: Self::Settings, overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Self {
        let (commands_tx, commands_rx) = tokio::sync::mpsc::channel(BUFFER_SIZE);
        let (events_tx, _) = tokio::sync::broadcast::channel(BUFFER_SIZE);
        let initial_peers = config.initial_peers.clone();

        let mut swarm_handler =
            SwarmHandler::new(config, commands_tx.clone(), commands_rx, events_tx.clone());

        overwatch_handle.runtime().spawn(async move {
            swarm_handler.run(initial_peers).await;
        });

        Self {
            events_tx,
            commands_tx,
        }
    }

    async fn process(&self, msg: Self::Message) {
        if let Err(e) = self.commands_tx.send(msg).await {
            tracing::error!("failed to send command to nomos-libp2p: {e:?}");
        }
    }

    async fn subscribe(
        &mut self,
        kind: Self::EventKind,
    ) -> broadcast::Receiver<Self::NetworkEvent> {
        match kind {
            // Might be cleaner to use different channels depending on `kind`.
            // At the same time `events_tx` is common to all events. Maybe fine this way
            EventKind::Message | EventKind::SyncRequest => {
                tracing::debug!("processed subscription to incoming messages");
                self.events_tx.subscribe()
            }
        }
    }
}
