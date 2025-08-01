#[derive(Debug)]
pub enum Error {
    /// There were no peers to send a message to.
    NoPeers,
    /// The message being blend was already in the cache, hence we avoid
    /// propagating it again.
    DuplicateMessage,
}
