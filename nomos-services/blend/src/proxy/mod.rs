pub mod core;
pub mod edge;

use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use services_utils::wait_until_services_are_ready;

use crate::message::ServiceMessage;

const LOG_TARGET: &str = "blend::service::proxy";

pub struct BlendProxyService<CoreAdapter, EdgeAdapter, RuntimeServiceId>
where
    CoreAdapter: core::Adapter<RuntimeServiceId>,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(CoreAdapter, EdgeAdapter)>,
}

impl<CoreAdapter, EdgeAdapter, RuntimeServiceId> ServiceData
    for BlendProxyService<CoreAdapter, EdgeAdapter, RuntimeServiceId>
where
    CoreAdapter: core::Adapter<RuntimeServiceId>,
{
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<
        <CoreAdapter::Network as crate::core::network::NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
    >;
}

#[async_trait::async_trait]
impl<CoreAdapter, EdgeAdapter, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendProxyService<CoreAdapter, EdgeAdapter, RuntimeServiceId>
where
    CoreAdapter: core::Adapter<RuntimeServiceId> + Send,
    EdgeAdapter: edge::Adapter<RuntimeServiceId> + Send,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<
            crate::core::BlendService<
                CoreAdapter::Backend,
                CoreAdapter::NodeId,
                CoreAdapter::Network,
                RuntimeServiceId,
            >,
        > + AsServiceId<
            crate::edge::BlendService<
                EdgeAdapter::Backend,
                EdgeAdapter::NodeId,
                EdgeAdapter::BroadcastSettings,
                RuntimeServiceId,
            >,
        > + Debug
        + Display
        + Clone
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
            _phantom: PhantomData,
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

        let core_adapter = CoreAdapter::new(
            overwatch_handle
                .relay::<crate::core::BlendService<_, _, _, _>>()
                .await?,
        );
        let _edge_adapter = EdgeAdapter::new(
            overwatch_handle
                .relay::<crate::edge::BlendService<_, _, _, _>>()
                .await?,
        );

        status_updater.notify_ready();
        tracing::info!(
            target: LOG_TARGET,
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &self.service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            crate::core::BlendService<_, _, _, _>,
            crate::edge::BlendService<_, _, _, _>
        )
        .await?;

        // TODO: Receive new sessions and turn on/off core and edge adapters
        //       https://github.com/logos-co/nomos/issues/1464
        while let Some(msg) = inbound_relay.recv().await {
            // TODO: For now, we always relay messages to the core service.
            //       Implement the proxy once we implement the membership updates.
            //       https://github.com/logos-co/nomos/issues/1464
            core_adapter.relay(msg).await;
        }

        Ok(())
    }
}
