use std::io;

use libp2p::{swarm::ConnectionId, PeerId};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No peers to send a message to")]
    NoPeers,
    #[error("IO error from peer {peer_id:?} connection {connection_id:?}: {error}")]
    PeerIOError {
        error: io::Error,
        peer_id: PeerId,
        connection_id: ConnectionId,
    },
}
