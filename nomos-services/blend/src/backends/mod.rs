#[cfg(feature = "libp2p")]
pub mod libp2p;

use std::{fmt::Debug, pin::Pin};

use futures::Stream;
use nomos_blend::membership::Membership;
use overwatch::overwatch::handle::OverwatchHandle;
use rand::RngCore;

use crate::BlendConfig;

/// A trait for blend backends that send messages to the blend network.
#[async_trait::async_trait]
pub trait BlendBackend<RuntimeServiceId> {
    type Settings: Clone + Debug + Send + Sync + 'static;
    type NodeId: Clone + Debug + Send + Sync + 'static;

    fn new<R>(
        service_config: BlendConfig<Self::Settings, Self::NodeId>,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        membership: Membership<Self::NodeId>,
        rng: R,
    ) -> Self
    where
        R: RngCore + Send + 'static;
    fn shutdown(&mut self);
    /// Publish a message to the blend network.
    async fn publish(&self, msg: Vec<u8>);
    /// Listen to messages received from the blend network.
    fn listen_to_incoming_messages(&mut self) -> Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>;
}
