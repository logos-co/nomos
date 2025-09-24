use std::{hash::Hash, marker::PhantomData};

use nomos_blend_scheduling::message_blend::{
    ProofsGenerator as ProofsGeneratorTrait, crypto::send::SessionCryptographicProcessor,
};
use nomos_utils::blake_rng::BlakeRng;
use overwatch::overwatch::OverwatchHandle;
use rand::SeedableRng as _;

use crate::{
    edge::{LOG_TARGET, Settings, backends::BlendBackend},
    session::SessionInfo,
};

pub struct MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId> {
    cryptographic_processor: SessionCryptographicProcessor<NodeId, ProofsGenerator>,
    backend: Backend,
    _phantom: PhantomData<RuntimeServiceId>,
}

impl<Backend, NodeId, ProofsGenerator, RuntimeServiceId>
    MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Send + 'static,
    ProofsGenerator: ProofsGeneratorTrait,
{
    /// Creates a [`MessageHandler`] with the given membership.
    ///
    /// It returns [`Error`] if the membership does not satisfy the following
    /// edge node condition:
    /// 1. The membership size is at least `settings.minimum_network_size`.
    /// 2. The local node is not a core node.
    pub fn try_new_with_edge_condition_check(
        settings: &Settings<Backend, NodeId, RuntimeServiceId>,
        session_info: SessionInfo<NodeId>,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    ) -> Result<Self, Error>
    where
        NodeId: Eq + Hash,
    {
        let membership_size = session_info.membership.size();
        if membership_size < settings.minimum_network_size.get() as usize {
            Err(Error::NetworkIsTooSmall(membership_size))
        } else if session_info.membership.contains_local() {
            Err(Error::LocalIsCoreNode)
        } else {
            Ok(Self::new(settings, session_info, overwatch_handle))
        }
    }

    fn new(
        settings: &Settings<Backend, NodeId, RuntimeServiceId>,
        SessionInfo {
            membership,
            poq_generation_and_verification_inputs: poq_inputs,
        }: SessionInfo<NodeId>,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    ) -> Self {
        let cryptographic_processor =
            SessionCryptographicProcessor::new(&settings.crypto, membership.clone(), poq_inputs);
        let backend = Backend::new(
            settings.backend.clone(),
            overwatch_handle,
            membership,
            BlakeRng::from_entropy(),
        );
        Self {
            cryptographic_processor,
            backend,
            _phantom: PhantomData,
        }
    }
}

impl<Backend, NodeId, ProofsGenerator, RuntimeServiceId>
    MessageHandler<Backend, NodeId, ProofsGenerator, RuntimeServiceId>
where
    NodeId: Eq + Hash + Clone + Send + 'static,
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Sync,
    ProofsGenerator: ProofsGeneratorTrait,
{
    /// Blend a new message received from another service.
    pub async fn handle_messages_to_blend(&mut self, message: Vec<u8>) {
        let Ok(message) = self
            .cryptographic_processor
            .encapsulate_data_payload(&message)
            .await
            .inspect_err(|e| {
                tracing::error!(target: LOG_TARGET, "Failed to encapsulate message: {e:?}");
            })
        else {
            return;
        };
        self.backend.send(message).await;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Network is too small: {0}")]
    NetworkIsTooSmall(usize),
    #[error("Local node is a core node")]
    LocalIsCoreNode,
}
