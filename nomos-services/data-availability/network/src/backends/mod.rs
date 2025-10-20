pub mod libp2p;
pub mod mock;

use std::{collections::HashSet, pin::Pin};

use ::libp2p::PeerId;
use futures::Stream;
use nomos_core::{
    da::BlobId,
    header::HeaderId,
    sdp::{ProviderId, SessionNumber},
};
use nomos_da_network_core::{
    addressbook::AddressBookHandler, protocols::sampling::opinions::OpinionEvent,
    swarm::BalancerStats,
};
use overwatch::{overwatch::handle::OverwatchHandle, services::state::ServiceState};
use subnetworks_assignations::MembershipHandler;
use tokio::sync::mpsc::UnboundedSender;

use super::Debug;

pub enum ConnectionStatus {
    Ready,
    InsufficientSubnetworkConnections,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ProcessingError {
    #[error("Network backend doesn't have enough subnetwork peers connected")]
    InsufficientSubnetworkConnections,
}

#[async_trait::async_trait]
pub trait NetworkBackend<RuntimeServiceId> {
    type Settings: Clone + Debug + Send + Sync + 'static;
    type State: ServiceState<Settings = Self::Settings> + Clone + Send + Sync;
    type Message: Debug + Send + Sync + 'static;
    type EventKind: Debug + Send + Sync + 'static;
    type NetworkEvent: Debug + Send + Sync + 'static;
    type Membership: MembershipHandler + Clone;
    type HistoricMembership: MembershipHandler + Clone;
    type Addressbook: AddressBookHandler + Clone;

    fn new(
        config: Self::Settings,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        membership: Self::Membership,
        addressbook: Self::Addressbook,
        subnet_refresh_signal: impl Stream<Item = ()> + Send + 'static,
        blancer_stats_sender: UnboundedSender<BalancerStats>,
        opinion_sender: UnboundedSender<OpinionEvent>,
    ) -> Self;
    fn shutdown(&mut self);
    async fn process(&self, msg: Self::Message);
    fn update_status(&mut self, status: ConnectionStatus);
    async fn subscribe(
        &mut self,
        event: Self::EventKind,
    ) -> Pin<Box<dyn Stream<Item = Self::NetworkEvent> + Send>>;
    async fn start_historic_sampling(
        &self,
        session_id: SessionNumber,
        block_id: HeaderId,
        blob_ids: HashSet<BlobId>,
        membership: Self::HistoricMembership,
    );

    fn local_peer_id(&self) -> (PeerId, ProviderId);
}
