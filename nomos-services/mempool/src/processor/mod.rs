pub mod noop;
pub mod tx;

use std::error::Error;

use futures::future::BoxFuture;
use overwatch::services::{ServiceData, relay::OutboundRelay};

pub type ProcessorTask<Error> = BoxFuture<'static, Result<(), Error>>;

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
    async fn process(
        &self,
        payload: &Self::Payload,
    ) -> Result<Vec<ProcessorTask<Self::Error>>, Vec<Self::Error>>;
}
