use cryptarchia_engine::Slot;
use libp2p::PeerId;
use nomos_core::header::HeaderId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncDirection {
    /// Request blocks from the given slot forward.
    Forward { slot: Slot },
    /// Request blocks from the given block backwards
    Backward { start_block: HeaderId, peer: PeerId },
}

/// Request message to a peer
#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncRequest {
    Blocks { direction: SyncDirection },
    TipSlot,
}

/// Response message from a peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncResponse {
    Block(Vec<u8>),
    TipSlot(Slot),
    End,
}
