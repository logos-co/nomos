pub mod mock;

use nomos_core::block::BlockNumber;
use overwatch::DynError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SdpBackendError {
    #[error("Other SDP backend error: {0}")]
    Other(#[from] DynError),
}

#[async_trait::async_trait]
pub trait SdpBackend {
    type Settings;
    type Block;

    fn init(settings: Self::Settings) -> Self;

    async fn process_new_block(
        &mut self,
        block_number: BlockNumber,
        block: Self::Block,
    ) -> Result<(), SdpBackendError>;

    async fn process_lib_block(
        &mut self,
        block_number: BlockNumber,
        block: Self::Block,
    ) -> Result<(), SdpBackendError>;
}
