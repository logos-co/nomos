pub mod adapters;

use std::{collections::HashSet, pin::Pin};

use futures::Stream;
use nomos_core::{block::SessionNumber, da::BlobId, header::HeaderId};
use nomos_da_network_service::{
    api::ApiAdapter,
    backends::{
        libp2p::common::{CommitmentsEvent, HistoricSamplingEvent, SamplingEvent},
        NetworkBackend,
    },
    NetworkService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use subnetworks_assignations::MembershipHandler;

#[async_trait::async_trait]
pub trait NetworkAdapter<RuntimeServiceId> {
    type Backend: NetworkBackend<RuntimeServiceId> + Send + 'static;
    type Settings: Clone;
    type Membership: MembershipHandler;
    type Storage;
    type MembershipAdapter;
    type ApiAdapter: ApiAdapter;

    async fn new(
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

    async fn start_sampling(&mut self, blob_id: BlobId) -> Result<(), DynError>;

    async fn request_historic_sampling(
        &self,
        session_id: SessionNumber,
        block_id: HeaderId,
        blob_ids: HashSet<BlobId>,
    ) -> Result<(), DynError>;

    async fn listen_to_sampling_messages(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = SamplingEvent> + Send>>, DynError>;

    async fn request_commitments(&self, blob_id: BlobId) -> Result<(), DynError>;

    async fn listen_to_commitments_messages(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = CommitmentsEvent> + Send>>, DynError>;

    async fn listen_to_historic_sampling_messages(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = HistoricSamplingEvent> + Send>>, DynError>;
}
