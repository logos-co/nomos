pub mod libp2p;
pub mod mock;

use std::{collections::HashMap, pin::Pin};

use ::libp2p::PeerId;
use futures::Stream;
use multiaddr::Multiaddr;
use nomos_core::{block::BlockNumber, da::BlobId};
use nomos_da_network_core::{addressbook::AddressBookHandler, protocols::sampling::SubnetsConfig};
use overwatch::{overwatch::handle::OverwatchHandle, services::state::ServiceState};
use subnetworks_assignations::MembershipHandler;

use super::Debug;

#[async_trait::async_trait]
pub trait NetworkBackend<RuntimeServiceId> {
    type Settings: Clone + Debug + Send + Sync + 'static;
    type State: ServiceState<Settings = Self::Settings> + Clone + Send + Sync;
    type Message: Debug + Send + Sync + 'static;
    type EventKind: Debug + Send + Sync + 'static;
    type NetworkEvent: Debug + Send + Sync + 'static;
    type Membership: MembershipHandler + Clone;
    type Addressbook: AddressBookHandler + Clone;

    fn new(
        config: Self::Settings,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        membership: Self::Membership,
        addressbook: Self::Addressbook,
        subnets_settings: SubnetsConfig,
    ) -> Self;
    fn shutdown(&mut self);
    async fn process(&self, msg: Self::Message);
    async fn subscribe(
        &mut self,
        event: Self::EventKind,
    ) -> Pin<Box<dyn Stream<Item = Self::NetworkEvent> + Send>>;
    async fn start_historic_sampling(
        &self,
        block_number: BlockNumber,
        blob_id: BlobId,
        membership: HashMap<PeerId, Multiaddr>,
    );
    fn local_peer_id(&self) -> PeerId;
}
