#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No peer to connect to")]
    NoPeers,
}
