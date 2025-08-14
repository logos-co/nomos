use libp2p::{swarm::ConnectionId, PeerId};

#[derive(Debug)]
pub enum Error {
    /// There were no peers to send a message to.
    NoPeers,
    NoSessionForPeer {
        peer_id: PeerId,
        connection_id: ConnectionId,
    },
    InvalidMessage,
}
