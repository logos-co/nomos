use nomos_libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

pub use crate::backends::libp2p::swarm::{DiscoveryCommand, PubSubCommand};

#[derive(Debug)]
#[non_exhaustive]
pub enum NetworkCommand {
    Connect(Dial),
    Info { reply: oneshot::Sender<Libp2pInfo> },
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Command {
    PubSub(PubSubCommand),
    Discovery(DiscoveryCommand),
    Network(NetworkCommand),
}

#[derive(Debug)]
pub struct Dial {
    pub addr: Multiaddr,
    pub retry_count: usize,
    pub result_sender: oneshot::Sender<Result<PeerId, nomos_libp2p::DialError>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Libp2pInfo {
    pub listen_addresses: Vec<Multiaddr>,
    pub n_peers: usize,
    pub n_connections: u32,
    pub n_pending_connections: u32,
}
