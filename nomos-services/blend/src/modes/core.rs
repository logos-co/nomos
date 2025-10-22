use std::fmt::{Debug, Display};

use overwatch::{
    overwatch::OverwatchHandle,
    services::{AsServiceId, ServiceData},
};
use tokio::sync::oneshot;
use tracing::{debug, error, warn};

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

    /// Shuts down the core service.
    ///
    /// If there is an ongoing session transition, it will be terminated
    /// immediately without being finished.
    ///
    /// This should be used when the node is transitioning to a new session (not
    /// core role) before the session transition period has passed, because
    /// the cleanup for the outdated session transition is not necessary.
    //
    // TODO: Eliminate this method by fundamentally preventing the exceptional
    // case where a new session starts before the previous session transition
    // has passed.
    pub async fn shutdown(self) {
        warn!(
            target = LOG_TARGET,
            "Shutting down the core service immediately without finishing the session transition"
        );
        self.shutdown_service().await;
    }

    /// Notifies the core service that the session transition period has passed
    /// and waits until it completes any cleanup. After that, it shuts down the
    /// service.
    ///
    /// This must be called instead of [`Self::shutdown`] when shutting down the
    /// core service because the node is not in the core role in the current
    /// session.
    pub async fn shutdown_after_finishing_session_transition(self) {
        debug!(
            target = LOG_TARGET,
            "Finishing the ongoing session transition before shutting down the core service"
        );
        if let Err(e) = self.finish_session_transition().await {
            error!(
                target = LOG_TARGET,
                "Failed to finish the existing session transition period during graceful shutdown. Shutting down anyway...: {e}"
            );
        }
        self.shutdown_service().await;
    }

    async fn finish_session_transition(&self) -> Result<(), Error> {
        let (reply_sender, reply_receiver) = oneshot::channel();
        self.0
            .handle_inbound_message(CoreServiceMessage::FinishSessionTransition { reply_sender })
            .await?;
        Ok(reply_receiver.await?)
    }

    async fn shutdown_service(self) {
        self.0.shutdown().await;
    }
}
