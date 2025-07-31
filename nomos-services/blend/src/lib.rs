use libp2p::PeerId;
use message::ServiceMessage;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use services_utils::wait_until_services_are_ready;
use std::{
    fmt::{Debug, Display},
    time::Duration,
};
use tracing::error;

pub mod core;
pub mod edge;
pub mod message;
pub mod settings;

const LOG_TARGET: &str = "blend::service";

pub struct BlendService<RuntimeServiceId> {
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
}

impl<RuntimeServiceId> ServiceData for BlendService<RuntimeServiceId> {
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<
        <core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as core::network::NetworkAdapter<
            RuntimeServiceId,
        >>::BroadcastSettings,
    >;
}

#[async_trait::async_trait]
impl<RuntimeServiceId> ServiceCore<RuntimeServiceId> for BlendService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<
            core::BlendService<
                core::backends::libp2p::Libp2pBlendBackend,
                PeerId,
                core::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
                RuntimeServiceId,
            >,
        > + AsServiceId<
            edge::BlendService<
                edge::backends::libp2p::Libp2pBlendBackend,
                PeerId,
                <core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as core::network::NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
                RuntimeServiceId,
            >,
        > + Debug
        + Display
        + Send
        + Sync
        + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
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
                    ref overwatch_handle,
                    ref status_updater,
                    ..
                },
            ..
        } = self;

        let core_relay = overwatch_handle
            .relay::<core::BlendService<_, _, _, _>>()
            .await
            .expect("");
        let _edge_relay = overwatch_handle
            .relay::<edge::BlendService<_, _, _, _>>()
            .await
            .expect("");

        status_updater.notify_ready();
        tracing::info!(
            target: LOG_TARGET,
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            overwatch_handle,
            Some(Duration::from_secs(60)),
            core::BlendService<_, _, _, _>,
            edge::BlendService<_, _, _, _>
        )
        .await?;

        // TODO: Receive new sessions and switch between core and edge services
        //       https://github.com/logos-co/nomos/issues/1464
        while let Some(msg) = inbound_relay.recv().await {
            if let Err(e) = core_relay.send(msg).await {
                error!(target: LOG_TARGET, "Failed to send message to core service: {e:?}");
            }
        }

        Ok(())
    }
}
