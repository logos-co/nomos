use bytes::Bytes;
use nomos_core::header::HeaderId;
use serde::{Deserialize, Serialize};

/// Blocks are serialized using nomos-core's wire format.
pub type SerialisedBlock = Bytes;

/// `HeaderId` is serialized using nomos-core's wire format.
pub type SerialisedHeaderId = Bytes;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum RequestMessage {
    /// A request to download blocks.
    DownloadBlocksRequest(DownloadBlocksRequest),
    /// A request to get the tip of the peer.
    GetTip,
}

/// A request to initiate block downloading from a peer.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadBlocksRequest {
    /// Return blocks up to `target_block`.
    pub target_block: HeaderId,
    /// The list of known blocks that the requester has.
    pub known_blocks: KnownBlocks,
}

/// A set of block identifiers the syncing peer already knows.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KnownBlocks {
    /// The local canonical chain latest block.
    pub local_tip: HeaderId,
    /// The latest immutable block.
    pub latest_immutable_block: HeaderId,
    /// The list of additional blocks that the requester has.
    pub additional_blocks: Vec<HeaderId>,
}

impl DownloadBlocksRequest {
    #[must_use]
    pub const fn new(
        target_block: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: Vec<HeaderId>,
    ) -> Self {
        Self {
            target_block,
            known_blocks: KnownBlocks {
                local_tip,
                latest_immutable_block,
                additional_blocks,
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum DownloadBlocksResponse {
    /// A response containing a block.
    Block(SerialisedBlock),
    /// A response indicating that no more blocks are available.
    NoMoreBlocks,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GetTipResponse {
    /// The tip of the peer.
    pub tip: Bytes,
}
