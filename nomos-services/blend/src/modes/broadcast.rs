use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use nomos_network::message::BackendNetworkMsg;
use overwatch::{
    overwatch::OverwatchHandle,
    services::{AsServiceId, ServiceData},
};
use services_utils::wait_until_services_are_ready;

use crate::{
    core::{network::NetworkAdapter, service_components::MessageComponents},
    modes::{Error, Mode},
};

pub struct BroadcastMode<Adapter, Message, RuntimeServiceId> {
    adapter: Adapter,
    _phantom: PhantomData<(Message, RuntimeServiceId)>,
}

impl<Adapter, Message, RuntimeServiceId> BroadcastMode<Adapter, Message, RuntimeServiceId>
where
    Adapter: NetworkAdapter<RuntimeServiceId> + Send + Sync,
{
    pub async fn new<Service>(
        overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    ) -> Result<Self, Error>
    where
        Service: ServiceData<Message = BackendNetworkMsg<Adapter::Backend, RuntimeServiceId>>,
        RuntimeServiceId: AsServiceId<Service> + Debug + Display + Send + Sync + 'static,
    {
        wait_until_services_are_ready!(&overwatch_handle, Some(Duration::from_secs(5)), Service)
            .await?;
        let relay = overwatch_handle.relay::<Service>().await?;
        let adapter = Adapter::new(relay);
        Ok(Self {
            adapter,
            _phantom: PhantomData,
        })
    }
}

#[async_trait::async_trait]
impl<Message, Adapter, RuntimeServiceId> Mode<Message>
    for BroadcastMode<Adapter, Message, RuntimeServiceId>
where
    Message: MessageComponents<
            Payload: Into<Vec<u8>>,
            BroadcastSettings: Into<Adapter::BroadcastSettings>,
        > + Send
        + Sync
        + 'static,
    Adapter: NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    RuntimeServiceId: Send + Sync + 'static,
{
    async fn handle_inbound_message(&self, message: Message) -> Result<(), Error> {
        let (payload, broadcast_settings) = message.into_components();
        self.adapter
            .broadcast(payload.into(), broadcast_settings.into())
            .await;
        Ok(())
    }

    async fn shutdown(self) {
        // No-op
    }
}

#[cfg(test)]
pub mod tests {
    use futures::StreamExt as _;
    use nomos_network::{backends::NetworkBackend, message::NetworkMsg, NetworkService};
    use overwatch::{
        overwatch::OverwatchRunner,
        services::{
            relay::OutboundRelay,
            state::{NoOperator, NoState},
            ServiceCore,
        },
        DynError, OpaqueServiceResourcesHandle,
    };
    use tokio::sync::{mpsc, oneshot};
    use tokio_stream::wrappers::BroadcastStream;
    use tracing::{debug, info};

    use super::*;

    #[test_log::test(test)]
    fn broadcast_mode() {
        let app = OverwatchRunner::<Services>::run(settings(), None).unwrap();
        app.runtime().handle().block_on(async {
            // Start the network service first.
            app.handle().start_all_services().await.unwrap();
            wait_until_services_are_ready!(
                &app.handle(),
                Some(Duration::from_secs(5)),
                TestNetworkService
            )
            .await
            .unwrap();

            // Create the BroadcastMode
            let mut mode =
                BroadcastMode::<TestNetworkAdapter, TestMessage, RuntimeServiceId>::new::<
                    TestNetworkService,
                >(app.handle())
                .await
                .unwrap();

            // Check if the mode broadcasts a message correctly.
            mode.handle_inbound_message(TestMessage(b"hello".to_vec()))
                .await
                .unwrap();
            assert_eq!(
                mode.adapter
                    .broadcasted_messages_receiver
                    .recv()
                    .await
                    .unwrap(),
                b"hello".to_vec()
            );

            // Check if the adapter is dropped on shutdown.
            let (drop_sender, drop_receiver) = oneshot::channel();
            mode.adapter.register_drop_notifier(drop_sender);
            mode.shutdown().await;
            drop_receiver.await.unwrap();

            // Check if the mode can be created again.
            let mut mode =
                BroadcastMode::<TestNetworkAdapter, TestMessage, RuntimeServiceId>::new::<
                    TestNetworkService,
                >(app.handle())
                .await
                .unwrap();
            mode.handle_inbound_message(TestMessage(b"world".to_vec()))
                .await
                .unwrap();
            assert_eq!(
                mode.adapter
                    .broadcasted_messages_receiver
                    .recv()
                    .await
                    .unwrap(),
                b"world".to_vec()
            );
        });
    }

    #[overwatch::derive_services]
    struct Services {
        network: TestNetworkService,
    }

    pub struct TestNetworkService {
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    }

    impl ServiceData for TestNetworkService {
        type Settings = ();
        type State = NoState<Self::Settings>;
        type StateOperator = NoOperator<Self::State>;
        type Message = BackendNetworkMsg<TestNetworkBackend, RuntimeServiceId>;
    }

    #[async_trait::async_trait]
    impl ServiceCore<RuntimeServiceId> for TestNetworkService {
        fn init(
            service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
            _: Self::State,
        ) -> Result<Self, DynError> {
            Ok(Self {
                service_resources_handle,
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
                debug!("Service {service_id} received message: {message:?}");
            }

            Ok(())
        }
    }

    pub struct TestNetworkBackend;

    #[async_trait::async_trait]
    impl<RuntimeServiceId> NetworkBackend<RuntimeServiceId> for TestNetworkBackend {
        type Settings = ();
        type Message = Vec<u8>;
        type PubSubEvent = ();
        type ChainSyncEvent = ();

        fn new((): Self::Settings, _: OverwatchHandle<RuntimeServiceId>) -> Self {
            Self
        }

        async fn process(&self, _: Self::Message) {}

        async fn subscribe_to_pubsub(&mut self) -> BroadcastStream<Self::PubSubEvent> {
            unimplemented!()
        }

        async fn subscribe_to_chainsync(&mut self) -> BroadcastStream<Self::ChainSyncEvent> {
            unimplemented!()
        }
    }

    pub struct TestNetworkAdapter {
        relay: OutboundRelay<
            <NetworkService<TestNetworkBackend, RuntimeServiceId> as ServiceData>::Message,
        >,
        broadcasted_messages_sender: mpsc::Sender<Vec<u8>>,
        broadcasted_messages_receiver: mpsc::Receiver<Vec<u8>>,
        drop_notifier: Option<oneshot::Sender<()>>,
    }

    impl Drop for TestNetworkAdapter {
        fn drop(&mut self) {
            if let Some(drop_notifier) = self.drop_notifier.take() {
                drop_notifier.send(()).unwrap();
            }
        }
    }

    #[async_trait::async_trait]
    impl<RuntimeServiceId> NetworkAdapter<RuntimeServiceId> for TestNetworkAdapter {
        type Backend = TestNetworkBackend;
        type BroadcastSettings = ();

        fn new(
            relay: OutboundRelay<
                <NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
            >,
        ) -> Self {
            let (broadcasted_messages_sender, broadcasted_messages_receiver) = mpsc::channel(100);
            Self {
                relay,
                broadcasted_messages_sender,
                broadcasted_messages_receiver,
                drop_notifier: None,
            }
        }

        async fn broadcast(&self, message: Vec<u8>, _: Self::BroadcastSettings) {
            debug!("Broadcasting message: {message:?}");
            self.relay
                .send(NetworkMsg::Process(message.clone()))
                .await
                .unwrap();
            self.broadcasted_messages_sender
                .send(message)
                .await
                .unwrap();
        }
    }

    impl TestNetworkAdapter {
        fn register_drop_notifier(&mut self, sender: oneshot::Sender<()>) {
            self.drop_notifier = Some(sender);
        }
    }

    #[derive(Debug)]
    pub struct TestMessage(Vec<u8>);

    impl MessageComponents for TestMessage {
        type Payload = Vec<u8>;
        type BroadcastSettings = ();

        fn into_components(self) -> (Self::Payload, Self::BroadcastSettings) {
            (self.0, ())
        }
    }

    fn settings() -> ServicesServiceSettings {
        ServicesServiceSettings { network: () }
    }
}
