use bytes::Bytes;
use serde::{Deserialize, Serialize};

pub type HeaderId = Bytes;
pub type Block = Bytes;

/// A request to initiate block downloading from a peer.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadBlocksRequest {
    /// Return blocks up to `target_block` if specified.
    pub target_block: Option<HeaderId>,
    /// The list of known blocks that the requester has.
    pub known_blocks: KnownBlocks,
}

/// A set of block identifiers the syncing peer already knows.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KnownBlocks {
    /// The latest block at the tip of the local chain.
    pub local_tip: HeaderId,
    /// The latest immutable block.
    pub latest_immutable_block: HeaderId,
    /// The list of additional blocks that the requester has.
    pub additional_blocks: Vec<HeaderId>,
}
