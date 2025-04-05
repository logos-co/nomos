#[async_trait::async_trait]
pub trait SdpLedgerBackend {
    type BlockNumber;
    type SdpMessage;
    type Error: std::error::Error + 'static + Send + Sync;
    
    async fn process_sdp_message(
        &mut self, 
        block_number: Self::BlockNumber, 
        message: Self::SdpMessage
    ) -> Result<(), Self::Error>;
    
    async fn mark_in_block(&mut self, block_number: Self::BlockNumber) -> Result<(), Self::Error>;
    fn discard_block(&mut self, block_number: Self::BlockNumber);
}