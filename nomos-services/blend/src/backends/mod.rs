#[cfg(feature = "libp2p")]
pub mod libp2p;

use std::{fmt::Debug, pin::Pin};

use futures::Stream;
use nomos_blend_message::encap::{DecapsulationOutput, EncapsulatedMessage};
use nomos_blend_scheduling::{membership::Membership, message_blend::crypto::ENCAPSULATION_COUNT};
use overwatch::overwatch::handle::OverwatchHandle;

use crate::BlendConfig;

/// A trait for blend backends that send messages to the blend network.
#[async_trait::async_trait]
pub trait BlendBackend<NodeId, Rng, RuntimeServiceId> {
    type Settings: Clone + Debug + Send + Sync + 'static;

    fn new(
        service_config: BlendConfig<Self::Settings, NodeId>,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        session_stream: Pin<Box<dyn Stream<Item = Membership<NodeId>> + Send>>,
        rng: Rng,
    ) -> Self;
    fn shutdown(&mut self);
    /// Publish a message to the blend network.
    async fn publish(&self, msg: EncapsulatedMessage<ENCAPSULATION_COUNT>);
    /// Listen to messages received from the blend network.
    fn listen_to_incoming_messages(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = DecapsulationOutput<ENCAPSULATION_COUNT>> + Send>>;
}
