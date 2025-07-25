use std::pin::Pin;

use futures::{Stream, StreamExt as _};
use kzgrs_backend::common::{build_blob_id, share::DaShare};
use libp2p::PeerId;
use nomos_da_network_core::SubnetworkId;
use overwatch::{overwatch::handle::OverwatchHandle, services::state::NoState};
use serde::{Deserialize, Serialize};
use subnetworks_assignations::MembershipHandler;
use tokio::sync::{
    broadcast::{self},
    mpsc,
};
use tokio_stream::wrappers::BroadcastStream;

use crate::{backends::NetworkBackend, DaAddressbook};

const BUFFER_SIZE: usize = 64;

#[derive(Debug)]
pub enum EventKind {
    Dispersal,
    Sample,
}

// A subset of dispersal protocol messages that will come over the wire.
// Imitates the message that will come from libp2p behaviour.
// Assuming that behaviour will wrap the nomos_da_message types.
#[derive(Debug, Clone)]
pub enum DisperseMessage {
    DispersalSuccess {
        blob_id: [u8; 32],
        subnetwork_id: u32,
    },
}

// A subset of sample protocol messages that will come over the wire.
// Imitates the message that will come from libp2p behaviour
// Assuming that behaviour will wrap the nomos_da_message types.
#[derive(Debug, Clone)]
pub enum SampleMessage {
    SampleSuccess {
        share: Box<DaShare>,
        subnetwork_id: u32,
    },
}

#[derive(Debug, Clone)]
pub enum Event {
    Disperse(DisperseMessage),
    Sample(SampleMessage),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MockConfig;

#[derive(Debug)]
pub enum Command {
    Disperse { share: DaShare, subnetwork_id: u32 },
}

#[derive(Clone)]
pub struct MockExecutorBackend {
    _config: MockConfig,
    _commands_tx: mpsc::Sender<Command>,
    events_tx: broadcast::Sender<Event>,
}

#[async_trait::async_trait]
impl<RuntimeServiceId> NetworkBackend<RuntimeServiceId> for MockExecutorBackend {
    type Settings = MockConfig;
    type State = NoState<MockConfig>;
    type Message = Command;
    type EventKind = EventKind;
    type NetworkEvent = Event;
    type Membership = MockMembership;
    type Addressbook = DaAddressbook;

    fn new(
        config: Self::Settings,
        _: OverwatchHandle<RuntimeServiceId>,
        _membership: Self::Membership,
        _addressbook: Self::Addressbook,
    ) -> Self {
        let (commands_tx, _) = mpsc::channel(BUFFER_SIZE);
        let (events_tx, _) = broadcast::channel(BUFFER_SIZE);
        Self {
            _config: config,
            _commands_tx: commands_tx,
            events_tx,
        }
    }

    fn shutdown(&mut self) {}

    async fn process(&self, msg: Self::Message) {
        match msg {
            Command::Disperse {
                share,
                subnetwork_id,
            } => {
                let blob_id = build_blob_id(&share.rows_commitments);

                let success_message = DisperseMessage::DispersalSuccess {
                    blob_id,
                    subnetwork_id,
                };

                let _ = self.events_tx.send(Event::Disperse(success_message));
            }
        }
    }

    async fn subscribe(
        &mut self,
        kind: Self::EventKind,
    ) -> Pin<Box<dyn Stream<Item = Self::NetworkEvent> + Send>> {
        match kind {
            EventKind::Dispersal | EventKind::Sample => Box::pin(
                BroadcastStream::new(self.events_tx.subscribe())
                    .filter_map(|event| async { event.ok() }),
            ),
        }
    }
}

#[derive(Clone)]
pub struct MockMembership;

impl MembershipHandler for MockMembership {
    type NetworkId = SubnetworkId;

    type Id = PeerId;

    fn membership(&self, _id: &Self::Id) -> std::collections::HashSet<Self::NetworkId> {
        todo!()
    }

    fn is_allowed(&self, _id: &Self::Id) -> bool {
        todo!()
    }

    fn members_of(&self, _network_id: &Self::NetworkId) -> std::collections::HashSet<Self::Id> {
        todo!()
    }

    fn members(&self) -> std::collections::HashSet<Self::Id> {
        todo!()
    }

    fn last_subnetwork_id(&self) -> Self::NetworkId {
        todo!()
    }

    fn subnetworks(
        &self,
    ) -> subnetworks_assignations::SubnetworkAssignations<Self::NetworkId, Self::Id> {
        todo!()
    }
}
