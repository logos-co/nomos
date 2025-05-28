#![allow(
    clippy::multiple_inherent_impl,
    reason = "We split the `Behaviour` impls into different modules for better code modularity."
)]

use std::error::Error;

use libp2p::{
    autonat, identify, identity, kad,
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour},
    PeerId,
};
use rand::RngCore;

use crate::{
    behaviour::gossipsub::compute_message_id, protocol_name::ProtocolName, AutonatClientSettings,
    IdentifySettings, KademliaSettings,
};

pub mod gossipsub;
pub mod kademlia;
pub mod nat;

// TODO: Risc0 proofs are HUGE (220 Kb) and it's the only reason we need to have
// this limit so large. Remove this once we transition to smaller proofs.
const DATA_LIMIT: usize = 1 << 18; // Do not serialize/deserialize more than 256 KiB

#[derive(Debug, Clone)]
pub enum BehaviourError {
    OperationNotSupported,
}

#[derive(NetworkBehaviour)]
pub struct Behaviour<R: Clone + Send + RngCore + 'static> {
    pub(crate) gossipsub: libp2p::gossipsub::Behaviour,
    // todo: support persistent store if needed
    pub(crate) kademlia: Toggle<kad::Behaviour<kad::store::MemoryStore>>,
    pub(crate) identify: Toggle<identify::Behaviour>,
    // The spec makes it mandatory to run an autonat server for a public node but in practice the
    // behaviour can be enabled all the time, because if the node ceases to be publicly reachable,
    // other peers will eventually not attempt to send dialback request to it.
    // The `Toggle` wrapper is used to disable the autonat server in special circumstances, for
    // example in specific tests.
    pub(crate) autonat_server: Toggle<autonat::v2::server::Behaviour<R>>,
    pub(crate) nat: Toggle<nat::NatBehaviour<R>>,
}

impl<R: Clone + Send + RngCore + 'static> Behaviour<R> {
    pub(crate) fn new(
        gossipsub_config: libp2p::gossipsub::Config,
        kad_config: Option<KademliaSettings>,
        identify_config: Option<IdentifySettings>,
        autonat_client_config: Option<AutonatClientSettings>,
        enable_autonat_server: bool,
        protocol_name: ProtocolName,
        public_key: identity::PublicKey,
        rng: R,
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

        let autonat_server = Toggle::from(
            enable_autonat_server.then_some(autonat::v2::server::Behaviour::new(rng.clone())),
        );

        let nat = Toggle::from(autonat_client_config.map(|autonat_client_config| {
            nat::NatBehaviour::new(rng, autonat_client_config.to_libp2p_config())
        }));

        Ok(Self {
            gossipsub,
            kademlia,
            identify,
            autonat_server,
            nat,
        })
    }
}
