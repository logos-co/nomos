use overwatch::overwatch::handle::OverwatchHandle;

#[async_trait::async_trait]
pub trait SdpBackend<RuntimeServiceId> {
    type BlockNumber: Clone + Send + Sync;
    type Message: Send + Sync;
    type Error: std::error::Error + 'static + Send + Sync;
    type Settings: Clone + Send + Sync + 'static;

    fn new(settings: Self::Settings, overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Self;

    async fn process_sdp_message(
        &mut self,
        block_number: Self::BlockNumber,
        message: Self::Message,
    ) -> Result<(), Self::Error>;

    async fn mark_in_block(&mut self, block_number: Self::BlockNumber) -> Result<(), Self::Error>;
    fn discard_block(&mut self, block_number: Self::BlockNumber);
}
