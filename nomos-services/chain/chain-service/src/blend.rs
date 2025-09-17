use std::marker::PhantomData;

use nomos_blend_service::message::{NetworkMessage, ServiceMessage};
use nomos_core::{block::Proposal, codec::SerdeOp};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use tracing::error;

use crate::LOG_TARGET;

pub struct BlendAdapter<BlendService>
where
    BlendService: ServiceData + nomos_blend_service::ServiceComponents,
{
    relay: OutboundRelay<<BlendService as ServiceData>::Message>,
    broadcast_settings: BlendService::BroadcastSettings,
    _phantom: PhantomData<BlendService>,
}

impl<BlendService> BlendAdapter<BlendService>
where
    BlendService: ServiceData + nomos_blend_service::ServiceComponents,
{
    pub const fn new(
        relay: OutboundRelay<<BlendService as ServiceData>::Message>,
        broadcast_settings: BlendService::BroadcastSettings,
    ) -> Self {
        Self {
            relay,
            broadcast_settings,
            _phantom: PhantomData,
        }
    }
}

impl<BlendService> BlendAdapter<BlendService>
where
    BlendService: ServiceData<Message = ServiceMessage<BlendService::BroadcastSettings>>
        + nomos_blend_service::ServiceComponents
        + Sync,
    <BlendService as ServiceData>::Message: Send,
    BlendService::BroadcastSettings: Clone + Sync,
{
    pub async fn publish_proposal(&self, block: Proposal) {
        if let Err((e, _)) = self
            .relay
            .send(ServiceMessage::Blend(NetworkMessage {
                message: <crate::messages::NetworkMessage as SerdeOp>::serialize(
                    &crate::messages::NetworkMessage::Proposal(block),
                )
                .expect("NetworkMessage should be able to be serialized")
                .to_vec(),
                broadcast_settings: self.broadcast_settings.clone(),
            }))
            .await
        {
            error!(target: LOG_TARGET, "Failed to relay proposal to blend service: {e:?}");
        }
    }
}
