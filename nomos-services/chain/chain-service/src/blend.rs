use std::marker::PhantomData;

use nomos_blend_service::message::{NetworkMessage, ServiceMessage};
use nomos_core::{block::Block, codec::SerdeOp};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::Serialize;
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
    pub async fn publish_block<Tx>(&self, block: Block<Tx>)
    where
        Tx: Clone + Eq + Serialize + for<'de> serde::Deserialize<'de> + Send,
    {
        if let Err((e, _)) = self
            .relay
            .send(ServiceMessage::Blend(NetworkMessage {
                message: <crate::messages::NetworkMessage<Tx> as SerdeOp>::serialize(
                    &crate::messages::NetworkMessage::Block(block),
                )
                .expect("NetworkMessage should be able to be serialized")
                .to_vec(),
                broadcast_settings: self.broadcast_settings.clone(),
            }))
            .await
        {
            error!(target: LOG_TARGET, "Failed to relay block to blend service: {e:?}");
        }
    }
}
