pub mod noop;
pub mod tx;

use std::{error::Error, time::Duration};

use futures::{future::BoxFuture, stream::FuturesUnordered};
use overwatch::services::{relay::OutboundRelay, ServiceData};

pub type SamplingFutureResult<Error> = BoxFuture<'static, Result<(), Error>>;

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
        trigger_sampling_tasks: &mut FuturesUnordered<SamplingFutureResult<Self::Error>>,
        trigger_sampling_delay: Duration,
        payload: &Self::Payload,
    ) -> Result<(), Vec<Self::Error>>;
}
