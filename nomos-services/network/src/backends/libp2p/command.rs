use std::collections::HashMap;

use nomos_libp2p::{libp2p::kad::PeerInfo, Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

#[derive(Debug)]
#[non_exhaustive]
pub enum PubSubCommand {
    Broadcast {
        topic: Topic,
        message: Box<[u8]>,
    },
    Subscribe(Topic),
    Unsubscribe(Topic),
    #[doc(hidden)]
    RetryBroadcast {
        topic: Topic,
        message: Box<[u8]>,
        retry_count: usize,
    },
}

#[derive(Debug)]
#[non_exhaustive]
pub enum DiscoveryCommand {
    GetClosestPeers {
        peer_id: PeerId,
        reply: oneshot::Sender<Vec<PeerInfo>>,
    },
    DumpRoutingTable {
        reply: oneshot::Sender<HashMap<u32, Vec<PeerId>>>,
    },
}

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

pub type Topic = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Libp2pInfo {
    pub listen_addresses: Vec<Multiaddr>,
    pub n_peers: usize,
    pub n_connections: u32,
    pub n_pending_connections: u32,
}
