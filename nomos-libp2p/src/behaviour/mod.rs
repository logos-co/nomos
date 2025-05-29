#![allow(
    clippy::multiple_inherent_impl,
    reason = "We split the `Behaviour` impls into different modules for better code modularity."
)]

use std::error::Error;

use cryptarchia_sync::ChainSyncError;
use libp2p::{
    autonat, identify, identity, kad,
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour},
    PeerId,
};
use rand::RngCore;
use thiserror::Error;

use crate::{
    behaviour::gossipsub::compute_message_id, protocol_name::ProtocolName, AutonatClientSettings,
    IdentifySettings, KademliaSettings,
};

pub mod chainsync;
pub mod gossipsub;
pub mod kademlia;
pub mod nat;

// TODO: Risc0 proofs are HUGE (220 Kb) and it's the only reason we need to have
// this limit so large. Remove this once we transition to smaller proofs.
const DATA_LIMIT: usize = 1 << 18; // Do not serialize/deserialize more than 256 KiB

pub(crate) struct BehaviourConfig {
    pub gossipsub_config: libp2p::gossipsub::Config,
    pub kademlia_config: Option<KademliaSettings>,
    pub identify_config: Option<IdentifySettings>,
    pub autonat_client_config: Option<AutonatClientSettings>,
    pub protocol_name: ProtocolName,
    pub public_key: identity::PublicKey,
}

#[derive(Debug, Error)]
pub enum BehaviourError {
    #[error("Operation not supported")]
    OperationNotSupported,
    #[error("Chainsync error: {0}")]
    ChainSyncError(#[from] ChainSyncError),
}

#[derive(NetworkBehaviour)]
pub struct Behaviour<R: Clone + Send + RngCore + 'static> {
    pub(crate) gossipsub: libp2p::gossipsub::Behaviour,
    // todo: support persistent store if needed
    pub(crate) kademlia: Toggle<kad::Behaviour<kad::store::MemoryStore>>,
    pub(crate) identify: Toggle<identify::Behaviour>,
    pub(crate) chain_sync: cryptarchia_sync::Behaviour,
    // The spec makes it mandatory to run an autonat server for a public node but in practice the
    // behaviour can be enabled all the time, because if the node ceases to be publicly reachable,
    // other peers will eventually not attempt to send dialback request to it.
    // The `Toggle` wrapper is used to disable the autonat server in special circumstances, for
    // example in specific tests.
    pub(crate) autonat_server: autonat::v2::server::Behaviour<R>,
    pub(crate) nat: Toggle<nat::NatBehaviour<R>>,
}

impl<R: Clone + Send + RngCore + 'static> Behaviour<R> {
    pub(crate) fn new(config: BehaviourConfig, rng: R) -> Result<Self, Box<dyn Error>> {
        let BehaviourConfig {
            gossipsub_config,
            kademlia_config: kad_config,
            identify_config,
            autonat_client_config,
            protocol_name,
            public_key,
        } = config;

        let peer_id = PeerId::from(public_key.clone());
        let gossipsub = libp2p::gossipsub::Behaviour::new(
            libp2p::gossipsub::MessageAuthenticity::Author(peer_id),
            libp2p::gossipsub::ConfigBuilder::from(gossipsub_config)
                .validation_mode(libp2p::gossipsub::ValidationMode::None)
                .message_id_fn(compute_message_id)
                .max_transmit_size(DATA_LIMIT)
                .build()?,
        )?;

        let identify = Toggle::from(identify_config.map(|identify_config| {
            identify::Behaviour::new(identify_config.to_libp2p_config(public_key, protocol_name))
        }));

        let kademlia = Toggle::from(kad_config.map(|kad_config| {
            kad::Behaviour::with_config(
                peer_id,
                kad::store::MemoryStore::new(peer_id),
                kad_config.to_libp2p_config(protocol_name),
            )
        }));

        let autonat_server = autonat::v2::server::Behaviour::new(rng.clone());

        let nat = Toggle::from(autonat_client_config.map(|autonat_client_config| {
            nat::NatBehaviour::new(rng, autonat_client_config.to_libp2p_config())
        }));

        let chain_sync = cryptarchia_sync::Behaviour::default();

        Ok(Self {
            gossipsub,
            kademlia,
            identify,
            chain_sync,
            autonat_server,
            nat,
        })
    }
}
