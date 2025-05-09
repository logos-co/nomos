mod behaviour;
mod config;
pub mod protocol_name;
mod swarm;

use blake2::{
    digest::{consts::U32, Digest as _},
    Blake2b,
};
pub use config::{secret_key_serde, IdentifySettings, KademliaSettings, SwarmConfig};
use libp2p::gossipsub::{Message, MessageId};
pub use libp2p::{
    self,
    core::upgrade,
    gossipsub::{self, PublishError, SubscriptionError},
    identity::{self, ed25519},
    swarm::{dial_opts::DialOpts, DialError, NetworkBehaviour, SwarmEvent},
    PeerId, SwarmBuilder, Transport,
};
pub use multiaddr::{multiaddr, Multiaddr, Protocol};

pub use crate::{behaviour::*, swarm::*};

fn compute_message_id(message: &Message) -> MessageId {
    let mut hasher = Blake2b::<U32>::new();
    hasher.update(&message.data);
    MessageId::from(hasher.finalize().to_vec())
}
