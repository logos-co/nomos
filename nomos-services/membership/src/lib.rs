use std::{
    collections::{BTreeSet, HashMap},
    fmt::{Debug, Display},
    pin::Pin,
    time::Duration,
};

use adapters::SdpAdapter;
use async_trait::async_trait;
use backends::{MembershipBackend, MembershipBackendError};
use futures::{Stream, StreamExt as _};
use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{Locator, ProviderId},
};
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use serde::{Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;

pub mod adapters;
pub mod backends;

pub type MembershipProviders = (SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>);

pub type MembershipSnapshotStream =
    Pin<Box<dyn Stream<Item = MembershipProviders> + Send + Sync + Unpin>>;

const BROADCAST_CHANNEL_SIZE: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MembershipServiceSettings<BackendSettings> {
    pub backend: BackendSettings,
}

pub enum MembershipMessage {
    Subscribe {
        service_type: nomos_core::sdp::ServiceType,
        result_sender: oneshot::Sender<Result<MembershipSnapshotStream, MembershipBackendError>>,
    },

    // This should be used only for testing purposes
    Update {
        block_number: BlockNumber,
        update_event: nomos_core::sdp::FinalizedBlockEvent,
    },
}

pub struct MembershipService<Backend, Sdp, RuntimeServiceId>
where
    Backend: MembershipBackend,
    Sdp: SdpAdapter,
    Backend::Settings: Clone,
{
    backend: Backend,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    subscribe_channels:
        HashMap<nomos_core::sdp::ServiceType, broadcast::Sender<MembershipProviders>>,
}

impl<Backend, Sdp, RuntimeServiceId> ServiceData
    for MembershipService<Backend, Sdp, RuntimeServiceId>
where
    Backend: MembershipBackend,
    Sdp: SdpAdapter,
    Backend::Settings: Clone,
{
    type Settings = MembershipServiceSettings<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = MembershipMessage;
}

#[async_trait]
impl<Backend, Sdp, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for MembershipService<Backend, Sdp, RuntimeServiceId>
where
    Backend: MembershipBackend + Send + Sync + 'static,
    Backend::Settings: Clone,

    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<Sdp::SdpService>
        + Clone
        + Display
        + Send
        + Sync
        + 'static
        + Debug,
    Sdp: SdpAdapter + Send + Sync + 'static,
    <<Sdp as SdpAdapter>::SdpService as ServiceData>::Message: 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        let MembershipServiceSettings {
            backend: backend_settings,
        } = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        Ok(Self {
            backend: Backend::init(backend_settings),
            service_resources_handle,
            subscribe_channels: HashMap::new(),
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
        let sdp_relay = self
            .service_resources_handle
            .overwatch_handle
            .relay::<Sdp::SdpService>()
            .await?;

        let sdp_adapter = Sdp::new(sdp_relay);
        let mut sdp_stream = sdp_adapter
            .finalized_blocks_stream()
            .await
            .map_err(|e| match e {
                adapters::SdpAdapterError::Other(error) => error,
            })?;

        self.service_resources_handle.status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &self.service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            <Sdp as SdpAdapter>::SdpService
        )
        .await?;

        loop {
            tokio::select! {
                Some(msg) = self.service_resources_handle.inbound_relay.recv()  => {
                    self.handle_message(msg).await;
                }
                Some(sdp_msg) = sdp_stream.next() => {
                     self.handle_sdp_update(sdp_msg).await;
                },
            }
        }
    }
}

impl<Backend, Sdp, RuntimeServiceId> MembershipService<Backend, Sdp, RuntimeServiceId>
where
    Backend: MembershipBackend,
    Sdp: SdpAdapter,
    Backend::Settings: Clone,
{
    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point"
    )]
    async fn handle_message(&mut self, msg: MembershipMessage) {
        match msg {
            MembershipMessage::Subscribe {
                service_type,
                result_sender,
            } => {
                tracing::info!(
                    "DEBUG: MembershipService: Subscribe request for service_type: {:?}",
                    service_type
                );

                let tx = self
                    .subscribe_channels
                    .entry(service_type)
                    .or_insert_with(|| {
                        let (tx, _) = broadcast::channel(BROADCAST_CHANNEL_SIZE);
                        tracing::info!("DEBUG: MembershipService: Created new broadcast channel for service_type: {:?}", service_type);
                        tx
                    });

                let stream = make_pin_broadcast_stream(tx.subscribe());
                let providers = self.backend.get_latest_providers(service_type).await;
                tracing::info!(
                    "DEBUG: MembershipService: Retrieved {} providers for service_type: {:?}",
                    providers.as_ref().map(|(_, p)| p.len()).unwrap_or(0),
                    service_type
                );

                if let Ok((session_id, providers)) = providers {
                    if !providers.is_empty() && tx.send((session_id, providers)).is_err() {
                        tracing::error!(
                            "Error sending initial membership snapshot for service type: {:?}",
                            service_type
                        );
                    }
                    if result_sender.send(Ok(stream)).is_err() {
                        tracing::error!(
                            "Error sending finalized updates receiver for service type: {:?}",
                            service_type
                        );
                    }
                    tracing::info!("DEBUG: MembershipService: Successfully sent subscription response for service_type: {:?}", service_type);
                } else {
                    tracing::error!(
                        "Failed to get latest providers for service type: {:?}",
                        service_type
                    );

                    if result_sender
                        .send(Err(MembershipBackendError::Other(
                            "Failed to get latest providers".into(),
                        )))
                        .is_err()
                    {
                        tracing::error!(
                            "Error sending error response for service type: {:?}",
                            service_type
                        );
                    }
                }
            }

            MembershipMessage::Update {
                block_number,
                update_event,
            } => {
                tracing::debug!(
                    "Received update for block number {}: {:?}",
                    block_number,
                    update_event
                );
                self.handle_sdp_update(update_event).await;
            }
        }
    }

    async fn handle_sdp_update(&mut self, sdp_msg: nomos_core::sdp::FinalizedBlockEvent) {
        match self.backend.update(sdp_msg).await {
            Ok(snapshot) => {
                if snapshot.is_none() {
                    // no new sessions
                    return;
                }

                let snapshot = snapshot.unwrap();

                // The list of all providers for each updated service type is sent to
                // appropriate subscribers per service type
                for (service_type, snapshot) in snapshot {
                    if let Some(tx) = self.subscribe_channels.get(&service_type) {
                        if tx.send(snapshot).is_err() {
                            tracing::error!("Error sending membership update");
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to update backend: {:?}", e);
            }
        }
    }
}

fn make_pin_broadcast_stream(
    receiver: broadcast::Receiver<MembershipProviders>,
) -> MembershipSnapshotStream {
    Box::pin(BroadcastStream::new(receiver).filter_map(|res| {
        Box::pin(async move {
            match res {
                Ok(update) => Some(update),
                Err(e) => {
                    tracing::warn!("Lagging Membership subscriber: {e:?}");
                    None
                }
            }
        })
    }))
}
