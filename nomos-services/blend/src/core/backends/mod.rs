#[cfg(feature = "libp2p")]
pub mod libp2p;

use std::{fmt::Debug, pin::Pin};

use futures::Stream;
use nomos_blend_message::crypto::proofs::{
    PoQVerificationInputsMinusSigningKey,
    quota::inputs::prove::public::{CoreInputs, LeaderInputs},
};
use nomos_blend_scheduling::{
    EncapsulatedMessage, membership::Membership,
    message_blend::crypto::IncomingEncapsulatedMessageWithValidatedPublicHeader,
    session::SessionEvent,
};
use overwatch::overwatch::handle::OverwatchHandle;

use crate::core::settings::BlendConfig;

pub type EpochInfo = LeaderInputs;

#[derive(Debug, Clone)]
pub struct PublicInfo<NodeId> {
    pub session: SessionInfo<NodeId>,
    pub epoch: EpochInfo,
}

impl<NodeId> From<PublicInfo<NodeId>> for PoQVerificationInputsMinusSigningKey {
    fn from(
        PublicInfo {
            epoch,
            session:
                SessionInfo {
                    core_public_inputs,
                    session,
                    ..
                },
        }: PublicInfo<NodeId>,
    ) -> Self {
        Self {
            core: core_public_inputs,
            leader: epoch,
            session,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionInfo<NodeId> {
    pub membership: Membership<NodeId>,
    pub session: u64,
    pub core_public_inputs: CoreInputs,
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
        current_public_info: PublicInfo<NodeId>,
        rng: Rng,
    ) -> Self;
    fn shutdown(self);
    /// Publish a message to the blend network.
    async fn publish(&self, msg: EncapsulatedMessage);
    /// Rotate session.
    async fn rotate_session(&mut self, new_session_info: SessionInfo<NodeId>);
    /// Complete the session transition.
    async fn complete_session_transition(&mut self);
    /// Rotate the epoch for the ongoing session.
    async fn rotate_epoch(&mut self, new_epoch_public_info: EpochInfo);
    /// Complete the epoch transition.
    async fn complete_epoch_transition(&mut self);
    /// Listen to messages received from the blend network.
    fn listen_to_incoming_messages(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = IncomingEncapsulatedMessageWithValidatedPublicHeader> + Send>>;
}
