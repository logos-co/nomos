use nomos_core::block::BlockNumber;

use crate::{SdpBackend, SdpBackendError};

pub struct MockSdpBackend;

#[async_trait::async_trait]
impl SdpBackend for MockSdpBackend {
    type Settings = ();
    type Block = ();

    fn init(_settings: Self::Settings) -> Self {
        unimplemented!()
    }

    async fn process_new_block(
        &mut self,
        _block_number: BlockNumber,
        _block: Self::Block,
    ) -> Result<(), SdpBackendError> {
        unimplemented!()
    }

    async fn process_lib_block(
        &mut self,
        _block_number: BlockNumber,
        _block: Self::Block,
    ) -> Result<(), SdpBackendError> {
        unimplemented!()
    }
}
