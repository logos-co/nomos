use nomos_libp2p::{libp2p::StreamProtocol, Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

#[derive(Debug)]
#[non_exhaustive]
pub enum Command {
    Connect(Dial),
    Broadcast {
        topic: Topic,
        message: Box<[u8]>,
    },
    Subscribe(Topic),
    Unsubscribe(Topic),
    Info {
        reply: oneshot::Sender<Libp2pInfo>,
    },
    #[doc(hidden)]
    RetryBroadcast {
        topic: Topic,
        message: Box<[u8]>,
        retry_count: usize,
    },
    StreamSend {
        peer_id: PeerId,
        protocol: StreamProtocol,
        message: Box<[u8]>,
    },
}

#[derive(Debug, Clone)]
pub struct Dial {
    pub addr: Multiaddr,
    pub retry_count: usize,
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
