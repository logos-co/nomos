use std::hash::Hash;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum NetworkMessage<Block>
where
    Block: Clone + Eq + Hash,
{
    Block(Block),
    SyncRequest(u64),
}
