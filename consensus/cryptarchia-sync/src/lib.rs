pub mod config;
#[cfg(feature = "libp2p")]
mod libp2p;
mod messages;

pub type DynError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub enum ProviderResponse<Response> {
    Available(Response),
    Unavailable { reason: String },
}
pub type TipResponse = ProviderResponse<GetTipResponse>;

pub type BlocksResponse = ProviderResponse<BoxStream<'static, Result<SerialisedBlock, DynError>>>;

pub use config::Config;
use futures::stream::BoxStream;
#[cfg(feature = "libp2p")]
pub use libp2p::{
    behaviour::{Behaviour, BoxedStream, Event},
    errors::{ChainSyncError, ChainSyncErrorKind},
};
pub use messages::{DownloadBlocksRequest, GetTipResponse, SerialisedBlock};
pub use nomos_core::header::HeaderId;
