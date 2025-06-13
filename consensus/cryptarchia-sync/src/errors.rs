use libp2p::PeerId;
use thiserror::Error;

use crate::packing::PackingError;

#[derive(Debug, Error)]
pub enum ChainSyncErrorKind {
    #[error("Failed to start chain sync: {0}")]
    StartSyncError(String),

    #[error("Failed to request tips: {0}")]
    RequestTipsError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Stream error: {0}")]
    OpenStreamError(#[from] libp2p_stream::OpenStreamError),

    #[error("Failed to unpack data from reader: {0}")]
    PackingError(#[from] PackingError),

    #[error("Failed to receive data from channel: {0}")]
    ChannelReceiveError(String),

    #[error("Failed to send data to channel: {0}")]
    ChannelSendError(String),
}

#[derive(Debug, Error)]
#[error("Peer {peer}: {kind}")]
pub struct ChainSyncError {
    pub peer: PeerId,
    #[source]
    pub kind: ChainSyncErrorKind,
}

impl ChainSyncError {
    #[must_use]
    pub const fn new(peer: PeerId, kind: ChainSyncErrorKind) -> Self {
        Self { peer, kind }
    }
}

impl From<(PeerId, std::io::Error)> for ChainSyncError {
    fn from((peer, err): (PeerId, std::io::Error)) -> Self {
        Self {
            peer,
            kind: err.into(),
        }
    }
}

impl From<(PeerId, libp2p_stream::OpenStreamError)> for ChainSyncError {
    fn from((peer, err): (PeerId, libp2p_stream::OpenStreamError)) -> Self {
        Self {
            peer,
            kind: err.into(),
        }
    }
}

impl From<(PeerId, PackingError)> for ChainSyncError {
    fn from((peer, err): (PeerId, PackingError)) -> Self {
        Self {
            peer,
            kind: err.into(),
        }
    }
}
