#[cfg(feature = "libp2p")]
mod libp2p;
mod messages;
mod packing;

#[cfg(feature = "libp2p")]
pub use libp2p::{
    behaviour::{Behaviour, BoxedStream, Event},
    errors::{ChainSyncError, ChainSyncErrorKind},
};
pub use messages::{DownloadBlocksRequest, GetTipResponse, SerialisedBlock};
pub use nomos_core::header::HeaderId;
