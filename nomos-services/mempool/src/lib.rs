pub mod backend;
pub mod da;
pub mod network;
pub mod tx;
pub mod verify;

use std::fmt::{Debug, Error, Formatter};

use backend::{MempoolError, Status};
pub use da::{service::DaMempoolService, settings::DaMempoolSettings};
use tokio::sync::oneshot::Sender;
pub use tx::{service::TxMempoolService, settings::TxMempoolSettings};

pub enum MempoolMsg<BlockId, Payload, Item, Key> {
    Add {
        payload: Payload,
        key: Key,
        reply_channel: Sender<Result<(), MempoolError>>,
    },
    View {
        ancestor_hint: BlockId,
        reply_channel: Sender<Box<dyn Iterator<Item = Item> + Send>>,
    },
    Prune {
        ids: Vec<Key>,
    },
    #[cfg(test)]
    BlockItems {
        block: BlockId,
        reply_channel: Sender<Option<Box<dyn Iterator<Item = Item> + Send>>>,
    },
    MarkInBlock {
        ids: Vec<Key>,
        block: BlockId,
    },
    Metrics {
        reply_channel: Sender<MempoolMetrics>,
    },
    Status {
        items: Vec<Key>,
        reply_channel: Sender<Vec<Status<BlockId>>>,
    },
}

impl<BlockId, Payload, Item, Key> Debug for MempoolMsg<BlockId, Payload, Item, Key>
where
    BlockId: Debug,
    Payload: Debug,
    Item: Debug,
    Key: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        match self {
            Self::View { ancestor_hint, .. } => {
                write!(f, "MempoolMsg::View {{ ancestor_hint: {ancestor_hint:?}}}")
            }
            Self::Add { payload, .. } => write!(f, "MempoolMsg::Add{{payload: {payload:?}}}"),
            Self::Prune { ids } => write!(f, "MempoolMsg::Prune{{ids: {ids:?}}}"),
            Self::MarkInBlock { ids, block } => {
                write!(
                    f,
                    "MempoolMsg::MarkInBlock{{ids: {ids:?}, block: {block:?}}}"
                )
            }
            #[cfg(test)]
            Self::BlockItems { block, .. } => {
                write!(f, "MempoolMsg::BlockItem{{block: {block:?}}}")
            }
            Self::Metrics { .. } => write!(f, "MempoolMsg::Metrics"),
            Self::Status { items, .. } => write!(f, "MempoolMsg::Status{{items: {items:?}}}"),
        }
    }
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct MempoolMetrics {
    pub pending_items: usize,
    pub last_item_timestamp: u64,
}
