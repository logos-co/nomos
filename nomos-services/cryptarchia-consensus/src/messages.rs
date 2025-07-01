use nomos_core::block::Block;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum NetworkMessage<Tx, Blob>
where
    Tx: Clone + Eq,
    Blob: Clone + Eq,
{
    Block(Block<Tx, Blob>),
}
