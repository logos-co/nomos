use std::fmt::{Debug, Display};

use overwatch::{
    overwatch::OverwatchHandle,
    services::{AsServiceId, ServiceData},
};
use tokio::sync::oneshot;
use tracing::error;

use crate::{
    core::message::ServiceMessage as CoreServiceMessage,
    modes::{Error, LOG_TARGET, ondemand::OnDemandServiceMode},
};

pub struct CoreMode<Service, Message, RuntimeServiceId>(
    OnDemandServiceMode<Service, RuntimeServiceId>,
)
where
    Service: ServiceData<Message = CoreServiceMessage<Message>>;

impl<Service, Message, RuntimeServiceId> CoreMode<Service, Message, RuntimeServiceId>
where
    Service: ServiceData<Message = CoreServiceMessage<Message>>,
    Message: Send + 'static,
    RuntimeServiceId: AsServiceId<Service> + Debug + Display + Send + Sync + 'static,
{
    pub async fn new(overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Result<Self, Error> {
        Ok(Self(OnDemandServiceMode::new(overwatch_handle).await?))
    }

    pub async fn handle_inbound_message(&self, message: Message) -> Result<(), Error> {
        self.0
            .handle_inbound_message(CoreServiceMessage::Common(message))
            .await
    }

    pub async fn shutdown(self) {
        self.0.shutdown().await;
    }

    pub async fn graceful_shutdown(self) {
        if let Err(e) = self.finish_session_transition_period().await {
            error!(
                target = LOG_TARGET,
                "Failed to finish the existing session transition period during graceful shutdown. Shutting down anyway...: {e}"
            );
        }
        self.shutdown().await;
    }

    async fn finish_session_transition_period(&self) -> Result<(), Error> {
        let (reply_sender, reply_receiver) = oneshot::channel();
        self.0
            .handle_inbound_message(CoreServiceMessage::FinishSessionTransitionPeriod {
                reply_sender,
            })
            .await?;
        Ok(reply_receiver.await?)
    }
}
