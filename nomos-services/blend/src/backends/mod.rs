#[cfg(feature = "libp2p")]
pub mod libp2p;

use std::{fmt::Debug, pin::Pin};

use futures::Stream;
use nomos_blend::membership::Membership;
use nomos_blend_message::sphinx::SphinxMessage;
use overwatch::overwatch::handle::OverwatchHandle;
use rand::RngCore;

/// A trait for blend backends that send messages to the blend network.
#[async_trait::async_trait]
pub trait BlendBackend<RuntimeServiceId> {
    type Settings: Clone + Debug + Send + Sync + 'static;
    type NodeId: Clone + Debug + Send + Sync + 'static;

    fn new<R>(
        config: Self::Settings,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        membership: Membership<Self::NodeId, SphinxMessage>,
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
