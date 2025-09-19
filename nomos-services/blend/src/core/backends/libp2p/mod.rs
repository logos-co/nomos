use std::pin::Pin;

use async_trait::async_trait;
use futures::{
    future::{AbortHandle, Abortable},
    Stream, StreamExt as _,
};
use libp2p::PeerId;
use nomos_blend_message::encap::ProofsVerifier as ProofsVerifierTrait;
use nomos_blend_scheduling::{
    message_blend::crypto::IncomingEncapsulatedMessageWithValidatedPublicHeader,
    EncapsulatedMessage,
};
use overwatch::overwatch::handle::OverwatchHandle;
use rand::RngCore;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::BroadcastStream;

use crate::core::{
    backends::{
        libp2p::{
            swarm::{BlendSwarm, BlendSwarmMessage, SwarmParams},
            tokio_provider::ObservationWindowTokioIntervalProvider,
        },
        BlendBackend, SessionInfo, SessionStream,
    },
    settings::BlendConfig,
};

const LOG_TARGET: &str = "blend::backend::libp2p";

pub(crate) mod behaviour;
pub mod settings;
pub use self::settings::Libp2pBlendBackendSettings;
mod swarm;
pub(crate) mod tokio_provider;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use self::tests::utils as core_swarm_test_utils;

/// A blend backend that uses the libp2p network stack.
pub struct Libp2pBlendBackend {
    swarm_task_abort_handle: AbortHandle,
    swarm_message_sender: mpsc::Sender<BlendSwarmMessage>,
    incoming_message_sender:
        broadcast::Sender<IncomingEncapsulatedMessageWithValidatedPublicHeader>,
}

const CHANNEL_SIZE: usize = 64;

#[async_trait]
impl<Rng, ProofsVerifier, RuntimeServiceId>
    BlendBackend<PeerId, Rng, ProofsVerifier, RuntimeServiceId> for Libp2pBlendBackend
where
    ProofsVerifier: ProofsVerifierTrait + Clone + Send + 'static,
    Rng: RngCore + Clone + Send + 'static,
{
    type Settings = Libp2pBlendBackendSettings;

    fn new(
        config: BlendConfig<Self::Settings>,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        SessionInfo {
            membership: current_membership,
            poq_verification_inputs: current_poq_verification_inputs,
        }: SessionInfo<PeerId>,
        session_stream: SessionStream<PeerId>,
        rng: Rng,
        proofs_verifier: ProofsVerifier,
    ) -> Self {
        let (swarm_message_sender, swarm_message_receiver) = mpsc::channel(CHANNEL_SIZE);
        let (incoming_message_sender, _) = broadcast::channel(CHANNEL_SIZE);
        let minimum_network_size = config.minimum_network_size.try_into().unwrap();

        let swarm =
            BlendSwarm::<_, _, _, ObservationWindowTokioIntervalProvider>::new(SwarmParams {
                config: &config,
                current_membership,
                current_poq_verification_inputs,
                incoming_message_sender: incoming_message_sender.clone(),
                minimum_network_size,
                proofs_verifier,
                rng,
                session_stream,
                swarm_message_receiver,
            });

        let (swarm_task_abort_handle, swarm_task_abort_registration) = AbortHandle::new_pair();
        overwatch_handle
            .runtime()
            .spawn(Abortable::new(swarm.run(), swarm_task_abort_registration));

        Self {
            swarm_task_abort_handle,
            swarm_message_sender,
            incoming_message_sender,
        }
    }

    fn shutdown(self) {
        drop(self);
    }

    async fn publish(&self, msg: EncapsulatedMessage) {
        if let Err(e) = self
            .swarm_message_sender
            .send(BlendSwarmMessage::Publish(msg))
            .await
        {
            tracing::error!(target: LOG_TARGET, "Failed to send message to BlendSwarm: {e}");
        }
    }

    fn listen_to_incoming_messages(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = IncomingEncapsulatedMessageWithValidatedPublicHeader> + Send>>
    {
        Box::pin(
            BroadcastStream::new(self.incoming_message_sender.subscribe())
                .filter_map(|event| async { event.ok() }),
        )
    }
}

impl Drop for Libp2pBlendBackend {
    fn drop(&mut self) {
        let Self {
            swarm_task_abort_handle,
            ..
        } = self;
        swarm_task_abort_handle.abort();
    }
}
