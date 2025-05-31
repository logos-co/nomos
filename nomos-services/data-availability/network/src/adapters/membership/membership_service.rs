use std::marker::PhantomData;

use async_trait::async_trait;
use nomos_membership::{
    adapters::SdpAdapter, backends::MembershipBackend, MembershipMessage, MembershipService,
    MembershipSnapshotStream,
};
use nomos_sdp_core::ServiceType;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use tokio::sync::oneshot;

use super::{MembershipAdapter, MembershipAdapterError};

pub struct MembershipServiceAdapter<Backend, Sdp, RuntimeServiceId> {
    relay: OutboundRelay<MembershipMessage>,
    phantom_backend: PhantomData<Backend>,
    phantom_sdp: PhantomData<Sdp>,
    phantom_runtime_service_id: PhantomData<RuntimeServiceId>,
}

#[async_trait]
impl<Backend, Sdp, RuntimeServiceId> MembershipAdapter
    for MembershipServiceAdapter<Backend, Sdp, RuntimeServiceId>
where
    RuntimeServiceId: Send + Sync + 'static,
    Sdp: SdpAdapter + Send + Sync + 'static,
    Backend: MembershipBackend + Send + Sync + 'static,
    Backend::Settings: Clone,
{
    type MembershipService = MembershipService<Backend, Sdp, RuntimeServiceId>;

    fn new(relay: OutboundRelay<<Self::MembershipService as ServiceData>::Message>) -> Self {
        Self {
            relay,
            phantom_runtime_service_id: PhantomData,
            phantom_backend: PhantomData,
            phantom_sdp: PhantomData,
        }
    }

    async fn subscribe(
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
