pub mod adapters;

use std::pin::Pin;

use futures::Stream;
use nomos_core::da::BlobId;
use nomos_da_network_core::SubnetworkId;
use nomos_da_network_service::{
    adapters::membership::MembershipAdapter,
    backends::{libp2p::common::SamplingEvent, NetworkBackend},
    NetworkService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};

#[async_trait::async_trait]
pub trait NetworkAdapter<RuntimeServiceId> {
    type Backend: NetworkBackend<RuntimeServiceId> + Send + 'static;
    type Membership: MembershipAdapter + Send + 'static;
    type Settings: Clone;

    async fn new(
        network_relay: OutboundRelay<
            <NetworkService<Self::Backend, Self::Membership, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;

    async fn start_sampling(
        &mut self,
        blob_id: BlobId,
        subnets: &[SubnetworkId],
    ) -> Result<(), DynError>;
    async fn listen_to_sampling_messages(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = SamplingEvent> + Send>>, DynError>;
}
