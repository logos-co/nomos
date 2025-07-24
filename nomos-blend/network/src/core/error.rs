use std::io;

use libp2p::{swarm::ConnectionId, PeerId};

#[derive(Debug)]
pub enum Error {
    /// There were no peers to send a message to.
    NoPeers,
    /// IO error from peer
    PeerIO {
        error: io::Error,
        peer_id: PeerId,
        connection_id: ConnectionId,
    },
    InvalidMessage,
}
