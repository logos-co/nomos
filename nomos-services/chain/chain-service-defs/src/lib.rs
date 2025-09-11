use cryptarchia_engine::Slot;
use nomos_core::header::HeaderId;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use tokio::sync::{broadcast, oneshot};

#[derive(Debug)]
pub enum ConsensusMsg<Block> {
    Info {
        tx: oneshot::Sender<CryptarchiaInfo>,
    },
    BlockSubscribe {
        sender: oneshot::Sender<broadcast::Receiver<Block>>,
    },
    GetHeaders {
        from: Option<HeaderId>,
        to: Option<HeaderId>,
        tx: oneshot::Sender<Vec<HeaderId>>,
    },
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptarchiaInfo {
    pub tip: HeaderId,
    pub slot: Slot,
    pub height: u64,
    pub mode: cryptarchia_engine::State,
}
