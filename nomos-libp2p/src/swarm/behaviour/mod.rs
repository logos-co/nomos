use std::error::Error;

use blake2::{Blake2b, Digest as _, digest::consts::U32};
use identify::settings::Settings as IdentifySettings;
use kademlia::settings::Settings as KademliaSettings;
use libp2p::{
    PeerId,
    gossipsub::{
        self as libp2p_gossipsub, ConfigBuilder, Message, MessageAuthenticity, MessageId,
        ValidationMode,
    },
    identify as libp2p_identify, identity, kad,
    swarm::{NetworkBehaviour, behaviour::toggle::Toggle},
};

use crate::protocol_name::ProtocolName;

pub mod gossipsub;
pub mod identify;
pub mod kademlia;

// TODO: Risc0 proofs are HUGE (220 Kb) and it's the only reason we need to have
// this limit so large. Remove this once we transition to smaller proofs.
const DATA_LIMIT: usize = 1 << 18; // Do not serialize/deserialize more than 256 KiB

#[derive(Debug, Clone)]
pub enum BehaviourError {
    OperationNotSupported,
}

#[derive(NetworkBehaviour)]
pub struct NomosP2pBehaviour {
    pub(crate) gossipsub: libp2p_gossipsub::Behaviour,
    // todo: support persistent store if needed
    pub(crate) kademlia: Toggle<kad::Behaviour<kad::store::MemoryStore>>,
    pub(crate) identify: Toggle<libp2p_identify::Behaviour>,
}

impl NomosP2pBehaviour {
    pub(crate) fn new(
        gossipsub_config: libp2p_gossipsub::Config,
        kad_config: Option<KademliaSettings>,
        identify_config: Option<IdentifySettings>,
        protocol_name: ProtocolName,
        public_key: identity::PublicKey,
    ) -> Result<Self, Box<dyn Error>> {
        let peer_id = PeerId::from(public_key.clone());
        let gossipsub = libp2p_gossipsub::Behaviour::new(
            MessageAuthenticity::Author(peer_id),
            ConfigBuilder::from(gossipsub_config)
                .validation_mode(ValidationMode::None)
                .message_id_fn(compute_message_id)
                .max_transmit_size(DATA_LIMIT)
                .build()?,
        )?;

        let identify = identify_config.map_or_else(
            || Toggle::from(None),
            |identify_config| {
                Toggle::from(Some(libp2p_identify::Behaviour::new(
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
}

fn compute_message_id(message: &Message) -> MessageId {
    let mut hasher = Blake2b::<U32>::new();
    hasher.update(&message.data);
    MessageId::from(hasher.finalize().to_vec())
}
