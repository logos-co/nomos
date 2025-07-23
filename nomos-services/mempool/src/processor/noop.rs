use std::{convert::Infallible, marker::PhantomData};

use overwatch::services::{relay::OutboundRelay, ServiceData};

use super::PayloadProcessor;

pub type NoOpPayloadProcessor<Service, Payload> = PhantomData<(Service, Payload)>;

#[async_trait::async_trait]
impl<Service, Payload> PayloadProcessor for NoOpPayloadProcessor<Service, Payload>
where
    Payload: Send + Sync,
    Service: ServiceData + Send + Sync,
{
    type Payload = Payload;
    type Settings = ();
    type Error = Infallible;

    type DaSamplingService = Service;

    fn new(
        (): Self::Settings,
        _: OutboundRelay<<Self::DaSamplingService as ServiceData>::Message>,
    ) -> Self {
        Self
    }

    async fn process(&self, _: &Self::Payload) -> Result<(), Vec<Self::Error>> {
        Ok(())
    }
}
