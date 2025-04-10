pub mod backend;
pub mod network;
pub mod service;

use std::fmt::{Debug, Error, Formatter};

use backend::{MempoolError, Status};
use tokio::sync::{
    broadcast,
    oneshot::{self, Sender},
};

pub trait TransactionId<Id> {
    fn id(&self) -> Id;
}

#[derive(Debug)]
pub enum ItemKind {
    Tx,
    Da,
    Sdp,
}

#[derive(Debug)]
pub enum MempoolItem<TxItem, DaItem, SdpItem> {
    Tx(TxItem),
    Da(DaItem),
    Sdp(SdpItem),
}

impl<Id, TxItem, DaItem, SdpItem> TransactionId<Id> for MempoolItem<TxItem, DaItem, SdpItem>
where
    TxItem: TransactionId<Id>,
    DaItem: TransactionId<Id>,
    SdpItem: TransactionId<Id>,
{
    fn id(&self) -> Id {
        match self {
            Self::Tx(tx) => tx.id(),
            Self::Da(da) => da.id(),
            Self::Sdp(sdp) => sdp.id(),
        }
    }
}

// Implements clone without deriving to avoid passing Clone bound to any
// MempoolItem users that does not require it to be clonable.
impl<TxItem, DaItem, SdpItem> Clone for MempoolItem<TxItem, DaItem, SdpItem>
where
    TxItem: Clone,
    DaItem: Clone,
    SdpItem: Clone,
{
    fn clone(&self) -> Self {
        match self {
            Self::Tx(tx) => Self::Tx(tx.clone()),
            Self::Da(da) => Self::Da(da.clone()),
            Self::Sdp(sdp) => Self::Sdp(sdp.clone()),
        }
    }
}

pub enum MempoolMsg<BlockId, Key, TxItem, DaItem, SdpItem> {
    Add {
        key: Key,
        item: MempoolItem<TxItem, DaItem, SdpItem>,
        reply_channel: Sender<Result<(), MempoolError>>,
    },
    ViewTxItems {
        ancestor_hint: BlockId,
        reply_channel: Sender<Box<dyn Iterator<Item = TxItem> + Send>>,
    },
    ViewDaItems {
        ancestor_hint: BlockId,
        reply_channel: Sender<Box<dyn Iterator<Item = DaItem> + Send>>,
    },
    ViewSdpItems {
        ancestor_hint: BlockId,
        reply_channel: Sender<Box<dyn Iterator<Item = SdpItem> + Send>>,
    },
    Prune {
        ids: Vec<Key>,
    },
    #[cfg(test)]
    BlockItems {
        block: BlockId,
        reply_channel:
            Sender<Option<Box<dyn Iterator<Item = MempoolItem<TxItem, DaItem, SdpItem>> + Send>>>,
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
    Subscribe {
        sender: oneshot::Sender<broadcast::Receiver<MempoolItem<TxItem, DaItem, SdpItem>>>,
    },
}

impl<BlockId, Key, TxItem, DaItem, SdpItem> Debug
    for MempoolMsg<BlockId, Key, TxItem, DaItem, SdpItem>
where
    BlockId: Debug,
    Key: Debug,
    TxItem: Debug,
    DaItem: Debug,
    SdpItem: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        match self {
            Self::ViewTxItems { ancestor_hint, .. }
            | Self::ViewDaItems { ancestor_hint, .. }
            | Self::ViewSdpItems { ancestor_hint, .. } => {
                write!(f, "MempoolMsg::View {{ ancestor_hint: {ancestor_hint:?}}}")
            }
            Self::Add { item, .. } => write!(f, "MempoolMsg::Add{{item: {item:?}}}"),
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
            Self::Subscribe { .. } => write!(f, "MempoolMsg::Subscribe"),
        }
    }
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct MempoolMetrics {
    pub pending_items: usize,
    pub last_item_timestamp: u64,
}
