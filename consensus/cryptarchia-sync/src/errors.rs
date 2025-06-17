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

#[derive(Debug, Error, Clone)]
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

// Implement Clone manually because some variants contain non-cloneable types
impl Clone for ChainSyncErrorKind {
    fn clone(&self) -> Self {
        match self {
            Self::IoError(e) => Self::IoError(std::io::Error::new(e.kind(), e.to_string())),
            Self::OpenStreamError(e) => match e {
                libp2p_stream::OpenStreamError::UnsupportedProtocol(p) => Self::OpenStreamError(
                    libp2p_stream::OpenStreamError::UnsupportedProtocol(p.clone()),
                ),
                libp2p_stream::OpenStreamError::Io(e) => {
                    Self::OpenStreamError(libp2p_stream::OpenStreamError::Io(std::io::Error::new(
                        e.kind(),
                        e.to_string(),
                    )))
                }
                err => Self::OpenStreamError(libp2p_stream::OpenStreamError::Io(
                    std::io::Error::other(err.to_string()),
                )),
            },
            Self::PackingError(e) => match e {
                PackingError::MessageTooLarge { max, actual } => {
                    Self::PackingError(PackingError::MessageTooLarge {
                        max: *max,
                        actual: *actual,
                    })
                }
                PackingError::Io(e) => Self::PackingError(PackingError::Io(std::io::Error::new(
                    e.kind(),
                    e.to_string(),
                ))),
                PackingError::Serialization(e) => {
                    Self::PackingError(PackingError::Serialization(e.clone()))
                }
            },
            err => err.clone(),
        }
    }
}
