pub mod libp2p;
pub mod mock;

use std::pin::Pin;

use futures::Stream;
use nomos_da_network_core::addressbook::AddressBookHandler;
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
    ) -> Self;
    fn shutdown(&mut self);
    async fn process(&self, msg: Self::Message);
    async fn subscribe(
        &mut self,
        event: Self::EventKind,
    ) -> Pin<Box<dyn Stream<Item = Self::NetworkEvent> + Send>>;
}
