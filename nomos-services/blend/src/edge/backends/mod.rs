#[cfg(feature = "libp2p")]
pub mod libp2p;

use std::pin::Pin;

use futures::Stream;
use nomos_blend_scheduling::{membership::Membership, EncapsulatedMessage};
use overwatch::overwatch::handle::OverwatchHandle;
use rand::RngCore;

/// A trait for blend backends that send messages to the blend network.
#[async_trait::async_trait]
pub trait BlendBackend<NodeId, RuntimeServiceId>
where
    NodeId: Clone,
{
    type Settings: Clone + Send + Sync + 'static;

    fn new<Rng>(
        settings: Self::Settings,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        session_stream: Pin<Box<dyn Stream<Item = Membership<NodeId>> + Send>>,
        current_membership: Option<Membership<NodeId>>,
        rng: Rng,
    ) -> Self
    where
        Rng: RngCore + Send + 'static;
    fn shutdown(self);
    /// Send a message to the blend network.
    async fn send(&self, msg: EncapsulatedMessage);
}
