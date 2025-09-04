pub mod backends;
pub(crate) mod service_components;
pub mod settings;

use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use backends::BlendBackend;
use futures::StreamExt as _;
use nomos_blend_scheduling::{
    membership::Membership, message_blend::crypto::CryptographicProcessor,
};
use nomos_core::wire;
use nomos_utils::blake_rng::BlakeRng;
use overwatch::{
    services::{
        resources::ServiceResourcesHandle,
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use rand::{RngCore, SeedableRng as _};
use serde::Serialize;
pub(crate) use service_components::ServiceComponents;
use settings::BlendConfig;
use tracing::{error, info};

use crate::message::ServiceMessage;

const LOG_TARGET: &str = "blend::service::edge";

/// Edge service that establishes a one-shot connection to one of core nodes
/// each time a message needs to be sent.
///
/// # Panics
/// This service should be started only when the following conditions are met.
/// Otherwise, it will panic during initialization.
/// - The Blend network is larger than the minimal network size.
/// - The local node is not one of the core nodes in the Blend network.
pub struct BlendService<Backend, NodeId, BroadcastSettings, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    membership: Membership<NodeId>,
}

impl<Backend, NodeId, BroadcastSettings, RuntimeServiceId> ServiceData
    for BlendService<Backend, NodeId, BroadcastSettings, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    type Settings = BlendConfig<Backend::Settings, NodeId>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<BroadcastSettings>;
}

#[async_trait::async_trait]
impl<Backend, NodeId, BroadcastSettings, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<Backend, NodeId, BroadcastSettings, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Send + Sync,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    BroadcastSettings: Serialize + Send,
    RuntimeServiceId: AsServiceId<Self> + Display + Debug + Clone + Send + Sync + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let membership = settings.membership();
        if membership.size() < settings.minimum_network_size.get() as usize {
            panic!("Blend network size is smaller than the required minimum. Edge service should not be started.");
        } else if membership.contains_local() {
            panic!("Local node is one of core nodes in the Blend network. Edge service should not be started.");
        }

        Ok(Self {
            service_resources_handle,
            membership,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_resources_handle:
                ServiceResourcesHandle {
                    inbound_relay,
                    overwatch_handle,
                    settings_handle,
                    status_updater,
                    ..
                },
            membership,
        } = self;

        let settings = settings_handle.notifier().get_updated_settings();

        let mut cryptoraphic_processor = CryptographicProcessor::new(
            settings.crypto.clone(),
            membership.clone(),
            BlakeRng::from_entropy(),
        );
        let mut messages_to_blend = inbound_relay.map(|ServiceMessage::Blend(message)| {
            wire::serialize(&message)
                .expect("Message from internal services should not fail to serialize")
        });
        let backend = <Backend as BlendBackend<NodeId, RuntimeServiceId>>::new(
            settings.backend,
            overwatch_handle.clone(),
            membership,
            BlakeRng::from_entropy(),
        );

        status_updater.notify_ready();
        info!(
            target: LOG_TARGET,
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        while let Some(message) = messages_to_blend.next().await {
            handle_messages_to_blend(message, &mut cryptoraphic_processor, &backend).await;
        }

        Ok(())
    }
}

/// Blend a new message received from another service.
async fn handle_messages_to_blend<NodeId, Rng, Backend, RuntimeServiceId>(
    message: Vec<u8>,
    cryptographic_processor: &mut CryptographicProcessor<NodeId, Rng>,
    backend: &Backend,
) where
    NodeId: Eq + Hash + Clone + Send,
    Rng: RngCore + Send,
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Sync,
{
    let Ok(message) = cryptographic_processor
        .encapsulate_data_payload(&message)
        .inspect_err(|e| {
            error!(target: LOG_TARGET, "Failed to encapsulate message: {e:?}");
        })
    else {
        return;
    };
    backend.send(message).await;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nomos_blend_message::crypto::Ed25519PrivateKey;
    use nomos_blend_scheduling::{
        membership::Node, message_blend::CryptographicProcessorSettings, EncapsulatedMessage,
    };
    use overwatch::overwatch::{OverwatchHandle, OverwatchRunner};
    use services_utils::wait_until_services_are_ready;

    use super::*;
    use crate::instance::tests::{key, nodes, NodeId};

    #[test]
    fn successful_init_as_edge_node() {
        let settings = settings(nodes(&[1]), key(99).0, 1);
        let app = OverwatchRunner::<Services>::run(settings, None).unwrap();
        app.runtime().handle().block_on(async {
            app.handle().start_all_services().await.unwrap();
            wait_until_services_are_ready!(app.handle(), Some(Duration::from_secs(1)), TestService)
                .await
                .unwrap();
        });
    }

    #[test]
    fn init_panics_if_core_node() {
        let settings = settings(nodes(&[1, 2]), key(1).0, 1);
        let app = OverwatchRunner::<Services>::run(settings, None).unwrap();
        app.runtime().handle().block_on(async {
            app.handle().start_all_services().await.unwrap();
            // Since the panic happens in a separate 'Overwatch' thread,
            // we cannot catch it here.
            // Instead, we check if the service fails to become ready.
            let result = wait_until_services_are_ready!(
                app.handle(),
                Some(Duration::from_secs(1)),
                TestService
            )
            .await;
            assert!(result.is_err());
        });
    }

    #[test]
    fn init_panics_if_network_is_small() {
        let settings = settings(nodes(&[1]), key(99).0, 2);
        let app = OverwatchRunner::<Services>::run(settings, None).unwrap();
        app.runtime().handle().block_on(async {
            app.handle().start_all_services().await.unwrap();
            // Since the panic happens in a separate 'Overwatch' thread,
            // we cannot catch it here.
            // Instead, we check if the service fails to become ready.
            let result = wait_until_services_are_ready!(
                app.handle(),
                Some(Duration::from_secs(1)),
                TestService
            )
            .await;
            assert!(result.is_err());
        });
    }

    #[overwatch::derive_services]
    struct Services {
        edge: TestService,
    }

    type TestService = BlendService<TestBackend, NodeId, BroadcastSettings, RuntimeServiceId>;

    struct TestBackend;

    #[async_trait::async_trait]
    impl BlendBackend<NodeId, RuntimeServiceId> for TestBackend {
        type Settings = ();

        fn new<Rng>(
            (): Self::Settings,
            _: OverwatchHandle<RuntimeServiceId>,
            _: Membership<NodeId>,
            _: Rng,
        ) -> Self
        where
            Rng: RngCore + Send + 'static,
        {
            Self
        }

        fn shutdown(self) {}

        async fn send(&self, _: EncapsulatedMessage) {}
    }

    type BroadcastSettings = ();

    fn settings(
        membership: Vec<Node<NodeId>>,
        signing_private_key: Ed25519PrivateKey,
        minimal_network_size: u64,
    ) -> ServicesServiceSettings {
        ServicesServiceSettings {
            edge: BlendConfig {
                backend: (),
                crypto: CryptographicProcessorSettings {
                    signing_private_key,
                    num_blend_layers: 1,
                },
                minimum_network_size: minimal_network_size.try_into().unwrap(),
                membership,
            },
        }
    }
}
