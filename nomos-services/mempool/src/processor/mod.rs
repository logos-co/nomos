pub mod noop;
pub mod tx;

use std::error::Error;

use overwatch::services::{relay::OutboundRelay, ServiceData};

#[async_trait::async_trait]
pub trait PayloadProcessor {
    type Payload;
    type Settings;
    type Error: Error;

    type DaSamplingService: ServiceData;

    fn new(
        settings: Self::Settings,
        outbound_relay: OutboundRelay<<Self::DaSamplingService as ServiceData>::Message>,
    ) -> Self;

    /// Executes required procedures before adding payload to the pool.
    async fn process(&self, payload: &Self::Payload) -> Result<(), Vec<Self::Error>>;
}
