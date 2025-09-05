use std::hash::Hash;

use nomos_blend_network::EncapsulatedMessageWithValidatedPublicHeader;
use nomos_blend_scheduling::{
    membership::Membership,
    message_blend::{crypto::CryptographicProcessor, CryptographicProcessorSettings},
    message_scheduler::{round_info::RoundInfo, MessageScheduler},
};
use nomos_utils::blake_rng::BlakeRng;
use rand::SeedableRng as _;
use serde::{Deserialize, Serialize};

use crate::{
    core::{
        backends::BlendBackend, handle_incoming_blend_message, handle_local_data_message,
        handle_release_round, network::NetworkAdapter,
    },
    message::{ProcessedMessage, ServiceMessage},
};

pub struct MessageHandler<NodeId> {
    cryptographic_processor: CryptographicProcessor<NodeId, BlakeRng>,
    rng: BlakeRng,
}

impl<NodeId> MessageHandler<NodeId>
where
    NodeId: Eq + Hash + Send,
{
    /// Creates a [`MessageHandler`] with the given membership.
    ///
    /// It returns [`Error`] if the membership does not satisfy the following
    /// core node condition:
    /// 1. The membership size is at least `settings.minimum_network_size`.
    /// 2. The local node is a core node.
    pub fn try_new_with_core_condition_check(
        settings: &CryptographicProcessorSettings,
        minimal_network_size: usize,
        membership: Membership<NodeId>,
    ) -> Result<Self, Error> {
        if membership.size() < minimal_network_size {
            Err(Error::NetworkIsTooSmall(membership.size()))
        } else if !membership.contains_local() {
            Err(Error::LocalIsNotCoreNode)
        } else {
            Ok(Self::new(settings, membership))
        }
    }

    fn new(settings: &CryptographicProcessorSettings, membership: Membership<NodeId>) -> Self {
        Self {
            cryptographic_processor: CryptographicProcessor::new(
                settings.clone(),
                membership,
                BlakeRng::from_entropy(),
            ),
            rng: BlakeRng::from_entropy(),
        }
    }

    pub async fn handle_local_data_message<
        Backend,
        BroadcastSettings,
        SessionClock,
        RuntimeServiceId,
    >(
        &mut self,
        message: ServiceMessage<BroadcastSettings>,
        backend: &Backend,
        scheduler: &mut MessageScheduler<
            SessionClock,
            BlakeRng,
            ProcessedMessage<BroadcastSettings>,
        >,
    ) where
        Backend: BlendBackend<NodeId, BlakeRng, RuntimeServiceId> + Sync,
        BroadcastSettings: Serialize,
    {
        handle_local_data_message(
            message,
            &mut self.cryptographic_processor,
            backend,
            scheduler,
        )
        .await;
    }

    pub fn handle_incoming_blend_message<SessionClock, BroadcastSettings>(
        &self,
        message: EncapsulatedMessageWithValidatedPublicHeader,
        scheduler: &mut MessageScheduler<
            SessionClock,
            BlakeRng,
            ProcessedMessage<BroadcastSettings>,
        >,
    ) where
        NodeId: Sync,
        SessionClock: Send,
        BroadcastSettings: for<'de> Deserialize<'de>,
    {
        handle_incoming_blend_message(message, scheduler, &self.cryptographic_processor);
    }

    pub async fn handle_release_round<Backend, NetAdapter, RuntimeServiceId>(
        &mut self,
        round_info: RoundInfo<ProcessedMessage<NetAdapter::BroadcastSettings>>,
        backend: &Backend,
        network_adapter: &NetAdapter,
    ) where
        Backend: BlendBackend<NodeId, BlakeRng, RuntimeServiceId> + Sync,
        NetAdapter: NetworkAdapter<RuntimeServiceId> + Sync,
    {
        handle_release_round(
            round_info,
            &mut self.cryptographic_processor,
            &mut self.rng,
            backend,
            network_adapter,
        )
        .await;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Network is too small: {0}")]
    NetworkIsTooSmall(usize),
    #[error("Local node is a core node")]
    LocalIsNotCoreNode,
}
