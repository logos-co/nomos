#![allow(
    clippy::multiple_inherent_impl,
    reason = "We split the `Behaviour` impls into different modules for better code modularity."
)]

use std::error::Error;

use cryptarchia_sync::ChainSyncError;
use libp2p::{identity, kad, swarm::NetworkBehaviour, PeerId};
use thiserror::Error;

use crate::{
    behaviour::gossipsub::compute_message_id, protocol_name::ProtocolName, IdentifySettings,
    KademliaSettings,
};

pub mod chainsync;
pub mod gossipsub;
pub mod kademlia;

// TODO: Risc0 proofs are HUGE (220 Kb) and it's the only reason we need to have
// this limit so large. Remove this once we transition to smaller proofs.
const DATA_LIMIT: usize = 1 << 18; // Do not serialize/deserialize more than 256 KiB

#[derive(Debug, Error)]
pub enum BehaviourError {
    #[error("Operation not supported")]
    OperationNotSupported,
    #[error("Chainsync error: {0}")]
    ChainSyncError(#[from] ChainSyncError),
}

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub(crate) gossipsub: libp2p::gossipsub::Behaviour,
    // todo: support persistent store if needed
    pub(crate) kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub(crate) identify: libp2p::identify::Behaviour,
    pub(crate) chain_sync: cryptarchia_sync::Behaviour,
}

impl Behaviour {
    pub(crate) fn new(
        gossipsub_config: libp2p::gossipsub::Config,
        kad_config: &KademliaSettings,
        identify_config: &IdentifySettings,
        chain_sync_config: cryptarchia_sync::Config,
        protocol_name: ProtocolName,
        public_key: identity::PublicKey,
    ) -> Result<Self, Box<dyn Error>> {
        let peer_id = PeerId::from(public_key.clone());
        let gossipsub = libp2p::gossipsub::Behaviour::new(
            libp2p::gossipsub::MessageAuthenticity::Author(peer_id),
            libp2p::gossipsub::ConfigBuilder::from(gossipsub_config)
                .validation_mode(libp2p::gossipsub::ValidationMode::None)
                .message_id_fn(compute_message_id)
                .max_transmit_size(DATA_LIMIT)
                .build()?,
        )?;

        let identify = libp2p::identify::Behaviour::new(
            identify_config.to_libp2p_config(public_key, protocol_name),
        );

        let kademlia = kad::Behaviour::with_config(
            peer_id,
            kad::store::MemoryStore::new(peer_id),
            kad_config.to_libp2p_config(protocol_name),
        );

        let chain_sync = cryptarchia_sync::Behaviour::new(chain_sync_config);

        Ok(Self {
            gossipsub,
            kademlia,
            identify,
            chain_sync,
        })
    }
}
