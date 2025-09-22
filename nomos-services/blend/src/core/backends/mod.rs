#[cfg(feature = "libp2p")]
pub mod libp2p;

use std::{fmt::Debug, pin::Pin};

use futures::Stream;
use nomos_blend_message::encap::encapsulated::PoQVerificationInputMinusSigningKey;
use nomos_blend_scheduling::{
    EncapsulatedMessage, membership::Membership,
    message_blend::crypto::IncomingEncapsulatedMessageWithValidatedPublicHeader,
    session::SessionEvent,
};
use overwatch::overwatch::handle::OverwatchHandle;

use crate::core::settings::BlendConfig;

pub struct SessionInfo<NodeId> {
    pub membership: Membership<NodeId>,
    pub poq_verification_inputs: PoQVerificationInputMinusSigningKey,
}

pub type SessionStream<NodeId> =
    Pin<Box<dyn Stream<Item = SessionEvent<SessionInfo<NodeId>>> + Send>>;

/// A trait for blend backends that send messages to the blend network.
#[async_trait::async_trait]
pub trait BlendBackend<NodeId, Rng, ProofsVerifier, RuntimeServiceId> {
    type Settings: Clone + Debug + Send + Sync + 'static;

    fn new(
        service_config: BlendConfig<Self::Settings>,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        current_session_info: SessionInfo<NodeId>,
        session_stream: SessionStream<NodeId>,
        rng: Rng,
        proofs_verifier: ProofsVerifier,
    ) -> Self;
    fn shutdown(self);
    /// Publish a message to the blend network.
    async fn publish(&self, msg: EncapsulatedMessage);
    /// Listen to messages received from the blend network.
    fn listen_to_incoming_messages(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = IncomingEncapsulatedMessageWithValidatedPublicHeader> + Send>>;
}
