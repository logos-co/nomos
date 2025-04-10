use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncPeerMessage {
    Block(Vec<u8>),
    TipData(u64),
    End,
}
