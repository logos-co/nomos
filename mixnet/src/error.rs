/// Mixnet Errors
#[derive(thiserror::Error, Debug)]
pub enum MixnetError {
    /// Invalid topology size
    #[error("invalid mixnet topology size")]
    InvalidTopologySize,
    /// Invalid packet flag
    #[error("invalid packet flag")]
    InvalidPacketFlag,
    /// Invalid fragment header
    #[error("invalid fragment header")]
    InvalidFragmentHeader,
    /// Invalid fragment set ID
    #[error("invalid fragment set ID: {0}")]
    InvalidFragmentSetId(#[from] uuid::Error),
    /// Invalid fragment ID
    #[error("invalid fragment ID")]
    InvalidFragmentId,
    /// Message too long
    #[error("message too long: {0} bytes")]
    MessageTooLong(usize),
    /// Invalid message
    #[error("invalid message")]
    InvalidMessage,
    /// Node address error
    #[error("node address error: {0}")]
    NodeAddressError(#[from] nym_sphinx_addressing::nodes::NymNodeRoutingAddressError),
    /// Sphinx packet error
    #[error("sphinx packet error: {0}")]
    SphinxPacketError(#[from] sphinx_packet::Error),
    /// Exponential distribution error
    #[error("exponential distribution error: {0}")]
    ExponentialError(#[from] rand_distr::ExpError),
}
