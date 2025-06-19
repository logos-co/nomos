use std::{convert::Infallible, marker::PhantomData};

use overwatch::services::{relay::OutboundRelay, state::NoState, ServiceData};

use super::PayloadProcessor;

pub struct NoSampling;

impl ServiceData for NoSampling {
    type Settings = ();
    type State = NoState<()>;
    type StateOperator = ();
    type Message = ();
}

pub type NoOpPayloadProcessor<Payload> = PhantomData<Payload>;

#[async_trait::async_trait]
impl<Payload> PayloadProcessor for NoOpPayloadProcessor<Payload>
where
    Payload: Send + Sync,
{
    type Payload = Payload;
    type Settings = ();
    type Error = Infallible;

    type DaSamplingService = NoSampling;

    fn new(
        (): Self::Settings,
        _: OutboundRelay<<Self::DaSamplingService as ServiceData>::Message>,
    ) -> Self {
        Self
    }

    async fn process(&self, _: &Self::Payload) -> Result<(), Self::Error> {
        Ok(())
    }
}
