use crate::gossipsub::ConfigBuilderError;
use libp2p::TransportError;

/// Local errors
#[derive(Debug)]
pub enum Error {
    /// Couldn't create gossip from parameters.
    BehaviourError(&'static str),
    /// See [`ConfigBuilderError`].
    ConfigBuilderError(ConfigBuilderError),
    /// See [`std::io::Error`].
    Io(std::io::Error),
    /// See [`TransportError`].
    TransportError(TransportError<std::io::Error>),
}

impl From<ConfigBuilderError> for Error {
    fn from(from: ConfigBuilderError) -> Self {
        Self::ConfigBuilderError(from)
    }
}

impl From<std::io::Error> for Error {
    fn from(from: std::io::Error) -> Self {
        Self::Io(from)
    }
}

impl From<TransportError<std::io::Error>> for Error {
    fn from(from: TransportError<std::io::Error>) -> Self {
        Self::TransportError(from)
    }
}
