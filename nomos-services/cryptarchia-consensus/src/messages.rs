use std::hash::Hash;

use nomos_core::block::Block;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "TODO: avoid unnecessary changes. Fix in separate PR"
)]
pub enum NetworkMessage<Tx, Blob>
where
    Tx: Clone + Eq + Hash,
    Blob: Clone + Eq + Hash,
{
    Block(Block<Tx, Blob>),
    SyncRequest(u64),
}
