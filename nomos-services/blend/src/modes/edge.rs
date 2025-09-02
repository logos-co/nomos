use std::fmt::{Debug, Display};

use overwatch::{
    overwatch::OverwatchHandle,
    services::{AsServiceId, ServiceData},
};

use crate::modes::{ondemand::OnDemandServiceMode, Error, Mode};

pub struct EdgeMode<Service, RuntimeServiceId>(OnDemandServiceMode<Service, RuntimeServiceId>)
where
    Service: ServiceData;

impl<Service, RuntimeServiceId> EdgeMode<Service, RuntimeServiceId>
where
    Service: ServiceData<Message: Send + 'static>,
    RuntimeServiceId: AsServiceId<Service> + Debug + Display + Send + Sync + 'static,
{
    pub async fn new(overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Result<Self, Error> {
        Ok(Self(OnDemandServiceMode::new(overwatch_handle).await?))
    }
}

#[async_trait::async_trait]
impl<Message, Service, RuntimeServiceId> Mode<Message> for EdgeMode<Service, RuntimeServiceId>
where
    Message: Send + 'static,
    Service: ServiceData<Message = Message> + 'static,
    RuntimeServiceId: AsServiceId<Service> + Debug + Display + Send + Sync + 'static,
{
    async fn handle_inbound_message(&self, message: Message) -> Result<(), Error> {
        self.0.handle_inbound_message(message).await
    }

    async fn shutdown(self) {
        self.0.shutdown().await;
    }
}
