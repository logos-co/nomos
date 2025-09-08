use std::fmt::{Debug, Display};

use overwatch::{
    overwatch::OverwatchHandle,
    services::{AsServiceId, ServiceData},
};

use crate::modes::{ondemand::OnDemandServiceMode, Error};

pub struct CoreMode<Service, RuntimeServiceId>(OnDemandServiceMode<Service, RuntimeServiceId>)
where
    Service: ServiceData;

impl<Service, RuntimeServiceId> CoreMode<Service, RuntimeServiceId>
where
    Service: ServiceData<Message: Send + 'static>,
    RuntimeServiceId: AsServiceId<Service> + Debug + Display + Send + Sync + 'static,
{
    pub async fn new(overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Result<Self, Error> {
        Ok(Self(OnDemandServiceMode::new(overwatch_handle).await?))
    }

    pub async fn handle_inbound_message(&self, message: Service::Message) -> Result<(), Error> {
        self.0.handle_inbound_message(message).await
    }

    pub async fn shutdown(self) {
        self.0.shutdown().await;
    }
}
