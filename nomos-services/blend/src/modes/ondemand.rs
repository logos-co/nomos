use std::{
    fmt::{Debug, Display},
    time::Duration,
};

use overwatch::{
    overwatch::OverwatchHandle,
    services::{relay::OutboundRelay, AsServiceId, ServiceData},
};
use services_utils::wait_until_services_are_ready;
use tracing::{error, info};

use crate::modes::{Error, LOG_TARGET};

pub struct OnDemandServiceMode<Service, RuntimeServiceId>
where
    Service: ServiceData,
{
    relay: OutboundRelay<Service::Message>,
    overwatch_handle: OverwatchHandle<RuntimeServiceId>,
}

impl<Service, RuntimeServiceId> OnDemandServiceMode<Service, RuntimeServiceId>
where
    Service: ServiceData<Message: Send + 'static>,
    RuntimeServiceId: AsServiceId<Service> + Debug + Display + Send + Sync + 'static,
{
    pub async fn new(overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Result<Self, Error> {
        let service_id = <RuntimeServiceId as AsServiceId<Service>>::SERVICE_ID;
        info!(target = LOG_TARGET, "Starting service {service_id:}");
        overwatch_handle
            .start_service::<Service>()
            .await
            .map_err(|e| Error::Overwatch(Box::new(e)))?;

        info!(
            target = LOG_TARGET,
            "Waiting until service {service_id:} is ready"
        );
        if let Err(e) =
            wait_until_services_are_ready!(&overwatch_handle, Some(Duration::from_secs(5)), Service)
                .await
        {
            stop_service(&overwatch_handle).await;
            return Err(e.into());
        }

        let relay = match overwatch_handle.relay::<Service>().await {
            Ok(relay) => relay,
            Err(e) => {
                stop_service(&overwatch_handle).await;
                return Err(e.into());
            }
        };

        Ok(Self {
            relay,
            overwatch_handle,
        })
    }

    pub async fn handle_inbound_message(&self, message: Service::Message) -> Result<(), Error> {
        self.relay.send(message).await.map_err(|(e, _)| e.into())
    }

    pub async fn shutdown(self) {
        stop_service(&self.overwatch_handle).await;
    }
}

async fn stop_service<Service, RuntimeServiceId>(
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
) where
    RuntimeServiceId: AsServiceId<Service> + Debug + Display + Sync,
{
    info!(
        target = LOG_TARGET,
        "Stopping service {}",
        <RuntimeServiceId as AsServiceId<Service>>::SERVICE_ID
    );
    if let Err(e) = overwatch_handle.stop_service::<Service>().await {
        error!(target = LOG_TARGET, "Failed to stop service: {e:}");
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt as _;
    use overwatch::{
        overwatch::OverwatchRunner,
        services::{
            state::{NoOperator, NoState},
            ServiceCore,
        },
        DynError, OpaqueServiceResourcesHandle,
    };
    use tokio::sync::oneshot;
    use tracing::{debug, info};

    use super::*;

    #[test_log::test(test)]
    fn ondemand_service_mode() {
        let app = OverwatchRunner::<Services>::run(settings(), None).unwrap();
        app.runtime().handle().block_on(async {
            let mode =
                OnDemandServiceMode::<PongService, RuntimeServiceId>::new(app.handle().clone())
                    .await
                    .unwrap();

            // Check if the mode has started by sending a Ping message.
            let (reply_sender, reply_receiver) = oneshot::channel();
            mode.handle_inbound_message(PongServiceMessage::Ping(100, reply_sender))
                .await
                .unwrap();
            let reply = reply_receiver.await.unwrap();
            assert_eq!(reply, 100);

            // Register a drop notifier to check if the service is dropped on shutdown.
            let (drop_sender, drop_receiver) = oneshot::channel();
            mode.handle_inbound_message(PongServiceMessage::RegisterDropNotifier(drop_sender))
                .await
                .unwrap();
            // Shutdown the mode.
            mode.shutdown().await;
            // Check if the service has been dropped.
            drop_receiver.await.unwrap();

            // Check if it can be started again.
            let mode =
                OnDemandServiceMode::<PongService, RuntimeServiceId>::new(app.handle().clone())
                    .await
                    .unwrap();
            let (reply_sender, reply_receiver) = oneshot::channel();
            mode.handle_inbound_message(PongServiceMessage::Ping(100, reply_sender))
                .await
                .unwrap();
            let reply = reply_receiver.await.unwrap();
            assert_eq!(reply, 100);
        });
    }

    #[overwatch::derive_services]
    struct Services {
        pong: PongService,
    }

    struct PongService {
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        drop_notifier: Option<oneshot::Sender<()>>,
    }

    impl Drop for PongService {
        fn drop(&mut self) {
            if let Some(drop_notifier) = self.drop_notifier.take() {
                drop_notifier.send(()).unwrap();
            }
        }
    }

    impl ServiceData for PongService {
        type Settings = ();
        type State = NoState<Self::Settings>;
        type StateOperator = NoOperator<Self::State>;
        type Message = PongServiceMessage;
    }

    enum PongServiceMessage {
        Ping(usize, oneshot::Sender<usize>),
        RegisterDropNotifier(oneshot::Sender<()>),
    }

    #[async_trait::async_trait]
    impl ServiceCore<RuntimeServiceId> for PongService {
        fn init(
            service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
            _: Self::State,
        ) -> Result<Self, DynError> {
            Ok(Self {
                service_resources_handle,
                drop_notifier: None,
            })
        }

        async fn run(mut self) -> Result<(), DynError> {
            let Self {
                service_resources_handle:
                    OpaqueServiceResourcesHandle::<Self, RuntimeServiceId> {
                        ref mut inbound_relay,
                        ref status_updater,
                        ..
                    },
                ..
            } = self;

            let service_id = <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID;
            status_updater.notify_ready();
            info!("Service {service_id} is ready.",);

            while let Some(message) = inbound_relay.next().await {
                match message {
                    PongServiceMessage::Ping(message, reply_sender) => {
                        debug!("Service {service_id} received message: {message}");
                        if reply_sender.send(message).is_err() {
                            error!("Failed to send response");
                        }
                    }
                    PongServiceMessage::RegisterDropNotifier(sender) => {
                        self.drop_notifier = Some(sender);
                        debug!("Service {service_id} registered drop notifier");
                    }
                }
            }

            Ok(())
        }
    }

    fn settings() -> ServicesServiceSettings {
        ServicesServiceSettings { pong: () }
    }
}
