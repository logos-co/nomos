use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use futures::StreamExt as _;
use nomos_network::NetworkService;
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceData},
};
use services_utils::wait_until_services_are_ready;
use tracing::info;

use crate::{
    // TODO: Move NetworkAdapter outside of core module.
    core::{mode_components::MessageComponents as _, network::NetworkAdapter},
    membership,
    message::ServiceMessage,
    mode::Mode,
    settings::CommonSettings,
};

const LOG_TARGET: &str = "blend::service::broadcast";

pub struct BroadcastMode<Network, MembershipAdapter> {
    _phantom: PhantomData<(Network, MembershipAdapter)>,
}

#[async_trait]
impl<Network, MembershipAdapter, RuntimeServiceId> Mode<RuntimeServiceId>
    for BroadcastMode<Network, MembershipAdapter>
where
    Network: NetworkAdapter<RuntimeServiceId, BroadcastSettings: Unpin> + Send + Sync + 'static,
    MembershipAdapter: membership::Adapter<NodeId: Send, Error: Send + Sync + 'static> + Send,
    membership::ServiceMessage<MembershipAdapter>: Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<NetworkService<Network::Backend, RuntimeServiceId>>
        + AsServiceId<<MembershipAdapter as membership::Adapter>::Service>
        + Debug
        + Display
        + Send
        + Sync
        + 'static,
{
    type Settings = CommonSettings;
    type Message = ServiceMessage<Network::BroadcastSettings>;

    async fn run<Service>(
        mut service_resource_handle: OpaqueServiceResourcesHandle<Service, RuntimeServiceId>,
    ) -> Result<OpaqueServiceResourcesHandle<Service, RuntimeServiceId>, DynError>
    where
        Service: ServiceData<
                Settings: Into<Self::Settings> + Clone + Send + Sync,
                State: Send + Sync,
                Message = Self::Message,
            >,
    {
        let OpaqueServiceResourcesHandle::<Service, RuntimeServiceId> {
            inbound_relay,
            overwatch_handle,
            settings_handle,
            status_updater,
            ..
        } = &mut service_resource_handle;

        let settings: Self::Settings = settings_handle.notifier().get_updated_settings().into();

        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _>,
            <MembershipAdapter as membership::Adapter>::Service
        )
        .await?;

        let network_adapter = async {
            let network_relay = overwatch_handle
                .relay::<NetworkService<_, _>>()
                .await
                .expect("Relay with network service should be available.");
            Network::new(network_relay)
        }
        .await;

        let mut session_stream = MembershipAdapter::new(
            overwatch_handle
                .relay::<<MembershipAdapter as membership::Adapter>::Service>()
                .await?,
            settings.crypto.non_ephemeral_signing_key.public_key(),
            // We don't need to generate secret zk info in the broadcast mode, so we ignore the
            // secret key at this level.
            None,
        )
        .subscribe()
        .await?;

        status_updater.notify_ready();
        info!(
            target: LOG_TARGET,
            "Blend service became ready by BroadcastMode"
        );

        let minimum_network_size = settings.minimum_network_size.get() as usize;
        loop {
            tokio::select! {
                Some(message) = inbound_relay.next() => {
                    let (payload, broadcast_settings) = message.into_components();
                    network_adapter.broadcast(payload, broadcast_settings).await;
                }
                Some(session) = session_stream.next() => {
                    if session.membership.size() >= minimum_network_size {
                        info!(
                            target: LOG_TARGET,
                            "Terminating the broadcast mode as the new membership is larger than the minimum network size ({} >= {}).",
                            session.membership.size(),
                            minimum_network_size,
                        );
                        return Ok(service_resource_handle);
                    }
                }
            }
        }
    }
}
