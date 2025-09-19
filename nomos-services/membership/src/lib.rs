use std::{
    collections::{BTreeSet, HashMap},
    fmt::{Debug, Display},
    marker::PhantomData,
    pin::Pin,
    time::Duration,
};

use async_trait::async_trait;
use backends::{MembershipBackend, MembershipBackendError};
use futures::{Stream, StreamExt as _};
use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{Locator, ProviderId},
};
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, NoState},
    },
};
use serde::{Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;

use crate::adapters::{
    sdp::{SdpAdapter, SdpAdapterError},
    storage::MembershipStorageAdapter,
};

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

pub struct MembershipService<Backend, Sdp, StorageAdapter, RuntimeServiceId>
where
    Backend: MembershipBackend,
    Sdp: SdpAdapter,
    Backend::Settings: Clone,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    subscribe_channels:
        HashMap<nomos_core::sdp::ServiceType, broadcast::Sender<MembershipProviders>>,

    phantom: PhantomData<Backend>,
}

impl<Backend, Sdp, StorageAdapter, RuntimeServiceId> ServiceData
    for MembershipService<Backend, Sdp, StorageAdapter, RuntimeServiceId>
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
impl<Backend, Sdp, StorageAdapter, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for MembershipService<Backend, Sdp, StorageAdapter, RuntimeServiceId>
where
    Backend: MembershipBackend<StorageAdapter = StorageAdapter> + Send + Sync + 'static,
    Backend::Settings: Clone,
    StorageAdapter: MembershipStorageAdapter + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<Sdp::SdpService>
        + AsServiceId<StorageAdapter::StorageService>
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
        Ok(Self {
            service_resources_handle,
            subscribe_channels: HashMap::new(),
            phantom: PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
        let MembershipServiceSettings {
            backend: backend_settings,
        } = self
            .service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let storage_service_relay = self
            .service_resources_handle
            .overwatch_handle
            .relay::<StorageAdapter::StorageService>()
            .await?;

        let storage_adapter = StorageAdapter::new(storage_service_relay);

        let mut backend = Backend::init(backend_settings, storage_adapter);

        let sdp_relay = self
            .service_resources_handle
            .overwatch_handle
            .relay::<Sdp::SdpService>()
            .await?;

        let sdp_adapter = Sdp::new(sdp_relay);
        let mut sdp_stream = sdp_adapter.lib_blocks_stream().await.map_err(|e| match e {
            SdpAdapterError::Other(error) => error,
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
                    self.handle_message(&mut backend,msg).await;
                }
                Some(sdp_msg) = sdp_stream.next() => {
                     self.handle_sdp_update(&mut backend, sdp_msg).await;
                },
            }
        }
    }
}

impl<Backend, Sdp, StorageAdapter, RuntimeServiceId>
    MembershipService<Backend, Sdp, StorageAdapter, RuntimeServiceId>
where
    Backend: MembershipBackend + Send + Sync + 'static,
    Sdp: SdpAdapter,
    Backend::Settings: Clone,
{
    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point"
    )]
    async fn handle_message(&mut self, backend: &mut Backend, msg: MembershipMessage) {
        match msg {
            MembershipMessage::Subscribe {
                service_type,
                result_sender,
            } => {
                let tx = self
                    .subscribe_channels
                    .entry(service_type)
                    .or_insert_with(|| {
                        let (tx, _) = broadcast::channel(BROADCAST_CHANNEL_SIZE);
                        tx
                    });

                let stream = make_pin_broadcast_stream(tx.subscribe());
                let providers = backend.get_latest_providers(service_type).await;

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
                self.handle_sdp_update(backend, update_event).await;
            }
        }
    }

    async fn handle_sdp_update(
        &self,
        backend: &mut Backend,
        sdp_msg: nomos_core::sdp::FinalizedBlockEvent,
    ) {
        match backend.update(sdp_msg).await {
            Ok(snapshot) => {
                if snapshot.is_none() {
                    // no new sessions
                    return;
                }

                let snapshot = snapshot.unwrap();

                // The list of all providers for each updated service type is sent to
                // appropriate subscribers per service type
                for (service_type, snapshot) in snapshot {
                    if let Some(tx) = self.subscribe_channels.get(&service_type)
                        && tx.send(snapshot).is_err()
                    {
                        tracing::error!("Error sending membership update");
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
