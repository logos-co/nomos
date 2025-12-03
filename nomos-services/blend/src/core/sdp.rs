use async_trait::async_trait;
use nomos_core::sdp::ActivityMetadata;
use overwatch::services::{
    ServiceData,
    relay::{OutboundRelay, RelayError},
};

/// A trait for communicating with the SDP service.
#[async_trait]
pub trait Adapter {
    type Service: ServiceData;

    fn new(relay: OutboundRelay<ServiceMessage<Self>>) -> Self;

    async fn post_activity(
        &self,
        proof: nomos_core::sdp::blend::ActivityProof,
    ) -> Result<(), RelayError>;
}

pub type ServiceMessage<SdpAdapter> = <<SdpAdapter as Adapter>::Service as ServiceData>::Message;

pub struct SdpServiceAdapter<Service>
where
    Service: ServiceData,
{
    relay: OutboundRelay<<Service as ServiceData>::Message>,
}

#[async_trait]
impl<Service> Adapter for SdpServiceAdapter<Service>
where
    Service: ServiceData<Message = nomos_sdp::SdpMessage>,
{
    type Service = Service;

    fn new(relay: OutboundRelay<ServiceMessage<Self>>) -> Self {
        Self { relay }
    }

    async fn post_activity(
        &self,
        proof: nomos_core::sdp::blend::ActivityProof,
    ) -> Result<(), RelayError> {
        let msg = nomos_sdp::SdpMessage::PostActivity {
            metadata: ActivityMetadata::Blend(Box::new(proof)),
        };
        self.relay.send(msg).await.map_err(|(e, _)| e)
    }
}
