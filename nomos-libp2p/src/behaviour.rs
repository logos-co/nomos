use std::{collections::HashMap, error::Error};

use libp2p::{
    gossipsub, identify, identity,
    kad::{self, QueryId},
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour},
    Multiaddr, PeerId,
};

use crate::{compute_message_id, protocol_name::ProtocolName, IdentifySettings, KademliaSettings};

// TODO: Risc0 proofs are HUGE (220 Kb) and it's the only reason we need to have
// this limit so large. Remove this once we transition to smaller proofs.
const DATA_LIMIT: usize = 1 << 18; // Do not serialize/deserialize more than 256 KiB

#[derive(Debug, Clone)]
pub enum BehaviourError {
    OperationNotSupported,
}

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub(crate) gossipsub: gossipsub::Behaviour,
    // todo: support persistent store if needed
    pub(crate) kademlia: Toggle<kad::Behaviour<kad::store::MemoryStore>>,
    pub(crate) identify: Toggle<identify::Behaviour>,
}

impl Behaviour {
    pub(crate) fn new(
        gossipsub_config: gossipsub::Config,
        kad_config: Option<KademliaSettings>,
        identify_config: Option<IdentifySettings>,
        protocol_name: ProtocolName,
        public_key: identity::PublicKey,
    ) -> Result<Self, Box<dyn Error>> {
        let peer_id = PeerId::from(public_key.clone());
        let gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Author(peer_id),
            gossipsub::ConfigBuilder::from(gossipsub_config)
                .validation_mode(gossipsub::ValidationMode::None)
                .message_id_fn(compute_message_id)
                .max_transmit_size(DATA_LIMIT)
                .build()?,
        )?;

        let identify = identify_config.map_or_else(
            || Toggle::from(None),
            |identify_config| {
                Toggle::from(Some(identify::Behaviour::new(
                    identify_config.to_libp2p_config(public_key, protocol_name),
                )))
            },
        );

        let kademlia = kad_config.map_or_else(
            || Toggle::from(None),
            |kad_config| {
                Toggle::from(Some(kad::Behaviour::with_config(
                    peer_id,
                    kad::store::MemoryStore::new(peer_id),
                    kad_config.to_libp2p_config(protocol_name),
                )))
            },
        );

        Ok(Self {
            gossipsub,
            kademlia,
            identify,
        })
    }

    pub fn kademlia_add_address(&mut self, peer_id: PeerId, addr: Multiaddr) {
        if let Some(kademlia) = self.kademlia.as_mut() {
            kademlia.add_address(&peer_id, addr);
        }
    }

    pub fn kademlia_routing_table_dump(&mut self) -> HashMap<u32, Vec<PeerId>> {
        self.kademlia
            .as_mut()
            .map_or_else(HashMap::new, |kademlia| {
                kademlia
                    .kbuckets()
                    .enumerate()
                    .map(|(bucket_idx, bucket)| {
                        let peers = bucket
                            .iter()
                            .map(|entry| *entry.node.key.preimage())
                            .collect::<Vec<_>>();
                        (bucket_idx as u32, peers)
                    })
                    .collect()
            })
    }

    pub fn get_kademlia_protocol_names(&self) -> Vec<String> {
        self.kademlia
            .as_ref()
            .map_or_else(std::vec::Vec::new, |kademlia| {
                kademlia
                    .protocol_names()
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect()
            })
    }

    pub fn kademlia_get_closest_peers(
        &mut self,
        peer_id: PeerId,
    ) -> Result<QueryId, BehaviourError> {
        self.kademlia.as_mut().map_or_else(
            || {
                tracing::error!("kademlia is not enabled");
                Err(BehaviourError::OperationNotSupported)
            },
            |kademlia| Ok(kademlia.get_closest_peers(peer_id)),
        )
    }

    pub fn bootstrap_kademlia(&mut self) {
        if let Some(kademlia) = self.kademlia.as_mut() {
            let res = kademlia.bootstrap();
            if let Err(e) = res {
                tracing::error!("failed to bootstrap kademlia: {e}");
            }
        }
    }
}
