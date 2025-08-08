#[derive(Debug)]
pub enum Error {
    /// There were no peers to send a message to.
    NoPeers,
    InvalidMessage,
}
