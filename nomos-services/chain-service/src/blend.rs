use std::marker::PhantomData;

use nomos_blend_service::{
    message::{NetworkMessage, ServiceMessage},
    ServiceExt as BlendServiceExt,
};
use nomos_core::{block::Block, wire};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::Serialize;
use tracing::error;

use crate::LOG_TARGET;

pub struct BlendAdapter<BlendService>
where
    BlendService: ServiceData + BlendServiceExt,
{
    relay: OutboundRelay<<BlendService as ServiceData>::Message>,
    broadcast_settings: <BlendService as BlendServiceExt>::BroadcastSettings,
    _phantom: PhantomData<BlendService>,
}

impl<BlendService> BlendAdapter<BlendService>
where
    BlendService: ServiceData + BlendServiceExt,
{
    pub const fn new(
        relay: OutboundRelay<<BlendService as ServiceData>::Message>,
        broadcast_settings: <BlendService as BlendServiceExt>::BroadcastSettings,
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
    BlendService: ServiceData<Message = ServiceMessage<<BlendService as BlendServiceExt>::BroadcastSettings>>
        + BlendServiceExt
        + Sync,
    <BlendService as ServiceData>::Message: Send,
    <BlendService as BlendServiceExt>::BroadcastSettings: Clone + Sync,
{
    pub async fn publish_block<Tx, BlobCert>(&self, block: Block<Tx, BlobCert>)
    where
        Tx: Clone + Eq + Serialize + Send,
        BlobCert: Clone + Eq + Serialize + Send,
    {
        if let Err((e, _)) = self
            .relay
            .send(ServiceMessage::Blend(NetworkMessage {
                message: wire::serialize(&crate::messages::NetworkMessage::Block(block)).unwrap(),
                broadcast_settings: self.broadcast_settings.clone(),
            }))
            .await
        {
            error!(target: LOG_TARGET, "Failed to relay block to blend service: {e:?}");
        }
    }
}
