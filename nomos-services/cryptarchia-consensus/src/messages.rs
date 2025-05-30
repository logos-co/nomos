use std::hash::Hash;

use nomos_core::block::Block;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum NetworkMessage<Tx>
where
    Tx: Clone + Eq + Hash,
{
    Block(Block<Tx>),
}
