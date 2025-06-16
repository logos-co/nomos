use std::{collections::HashMap, marker::PhantomData};

use libp2p::{Multiaddr, PeerId};
use nomos_da_network_core::SubnetworkId;
use nomos_membership::{MembershipMessage, MembershipService, MembershipSnapshotStream};
use nomos_sdp_core::ServiceType;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use subnetworks_assignations::MembershipCreator;
use tokio::sync::oneshot;

use crate::{
    membership::{handler::DaMembershipHandler, MembershipAdapter, MembershipAdapterError},
    storage::MembershipStorage,
};

pub struct MockMembershipAdapter<Backend, SdpAdapter, RuntimeServiceId>
where
    Backend: nomos_membership::backends::MembershipBackend,
    Backend::Settings: Clone,
    SdpAdapter: nomos_membership::adapters::SdpAdapter,
{
    relay: OutboundRelay<
        <MembershipService<Backend, SdpAdapter, RuntimeServiceId> as ServiceData>::Message,
    >,
    phantom: PhantomData<(Backend, SdpAdapter, RuntimeServiceId)>,
}

impl<Backend, SdpAdapter, RuntimeServiceId>
    MockMembershipAdapter<Backend, SdpAdapter, RuntimeServiceId>
where
    SdpAdapter: nomos_membership::adapters::SdpAdapter + Send + Sync + 'static,
    Backend: nomos_membership::backends::MembershipBackend + Send + Sync + 'static,
    Backend::Settings: Clone,
    RuntimeServiceId: Send + Sync + 'static,
{
    async fn subscribe_stream(
        &self,
        service_type: ServiceType,
    ) -> Result<MembershipSnapshotStream, MembershipAdapterError> {
        let (sender, receiver) = oneshot::channel();

        self.relay
            .send(MembershipMessage::Subscribe {
                result_sender: sender,
                service_type,
            })
            .await
            .map_err(|(e, _)| MembershipAdapterError::Other(e.into()))?;

        let res = receiver
            .await
            .map_err(|e| MembershipAdapterError::Other(e.into()))?
            .map_err(MembershipAdapterError::Backend);

        res
    }
}

#[async_trait::async_trait]
impl<Backend, SdpAdapter, RuntimeServiceId> MembershipAdapter
    for MockMembershipAdapter<Backend, SdpAdapter, RuntimeServiceId>
where
    SdpAdapter: nomos_membership::adapters::SdpAdapter + Send + Sync + 'static,
    Backend: nomos_membership::backends::MembershipBackend + Send + Sync + 'static,
    Backend::Settings: Clone,
    RuntimeServiceId: Send + Sync + 'static,
{
    type MembershipService = MembershipService<Backend, SdpAdapter, RuntimeServiceId>;

    fn new(relay: OutboundRelay<<Self::MembershipService as ServiceData>::Message>) -> Self {
        Self {
            phantom: PhantomData,
            relay,
        }
    }

    async fn subscribe(&self) -> Result<MembershipSnapshotStream, MembershipAdapterError> {
        self.subscribe_stream(ServiceType::DataAvailability).await
    }
}
