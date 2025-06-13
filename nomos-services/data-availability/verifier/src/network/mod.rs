pub mod adapters;

use futures::Stream;
use nomos_da_network_service::{backends::NetworkBackend, NetworkService};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use subnetworks_assignations::MembershipHandler;

#[async_trait::async_trait]
pub trait NetworkAdapter<RuntimeServiceId> {
    type Backend: NetworkBackend<RuntimeServiceId> + Send + 'static;
    type Settings;
    type Share;
    type Membership: MembershipHandler + Clone;
    type Storage;
    type MembershipAdapter;

    async fn new(
        settings: Self::Settings,
        network_relay: OutboundRelay<
            <NetworkService<
                Self::Backend,
                Self::Membership,
                Self::MembershipAdapter,
                Self::Storage,
                RuntimeServiceId,
            > as ServiceData>::Message,
        >,
    ) -> Self;

    async fn share_stream(&self) -> Box<dyn Stream<Item = Self::Share> + Unpin + Send>;
}
