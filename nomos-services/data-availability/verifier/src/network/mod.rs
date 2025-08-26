pub mod adapters;

use futures::Stream;
use nomos_da_network_service::{
    api::ApiAdapter,
    backends::{libp2p::common::BroadcastValidationResultSender, NetworkBackend},
    NetworkService,
};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use subnetworks_assignations::MembershipHandler;

pub struct ValidationRequest<T> {
    pub item: T,
    pub sender: BroadcastValidationResultSender,
}

#[async_trait::async_trait]
pub trait NetworkAdapter<RuntimeServiceId> {
    type Backend: NetworkBackend<RuntimeServiceId> + Send + 'static;
    type Settings;
    type Share;
    type Tx;
    type Membership: MembershipHandler + Clone;
    type Storage;
    type MembershipAdapter;
    type ApiAdapter: ApiAdapter;

    async fn new(
        settings: Self::Settings,
        network_relay: OutboundRelay<
            <NetworkService<
                Self::Backend,
                Self::Membership,
                Self::MembershipAdapter,
                Self::Storage,
                Self::ApiAdapter,
                RuntimeServiceId,
            > as ServiceData>::Message,
        >,
    ) -> Self;

    async fn share_stream(
        &self,
    ) -> Box<dyn Stream<Item = ValidationRequest<Self::Share>> + Unpin + Send>;
    async fn tx_stream(
        &self,
    ) -> Box<dyn Stream<Item = ValidationRequest<(u16, Self::Tx)>> + Unpin + Send>;
}
