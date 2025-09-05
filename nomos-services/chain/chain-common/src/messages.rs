use nomos_core::{block::Proposal, mantle::AuthenticatedMantleTx};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NetworkMessage<Tx>
where
    Tx: AuthenticatedMantleTx + Clone,
{
    Proposal(Proposal<Tx>),
}
