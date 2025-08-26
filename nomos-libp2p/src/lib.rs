pub mod behaviour;
mod config;
mod error;
pub mod protocol_name;
mod swarm;

pub use crate::swarm::*;
pub use config::{secret_key_serde, IdentifySettings, KademliaSettings, SwarmConfig};
pub use error::Error;
pub use libp2p::{
    self,
    core::upgrade,
    gossipsub::{self, PublishError, SubscriptionError},
    identity::{self, ed25519},
    swarm::{dial_opts::DialOpts, DialError, NetworkBehaviour, SwarmEvent},
    PeerId, SwarmBuilder, Transport,
};
pub use multiaddr::{multiaddr, Multiaddr, Protocol};

/// Alias that uses the local error
pub type Result<T> = core::result::Result<T, Error>;
