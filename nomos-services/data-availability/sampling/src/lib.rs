pub mod backend;
pub mod network;
pub mod storage;

use std::{collections::BTreeSet, fmt::Debug};

use backend::{DaSamplingServiceBackend, SamplingState};
use kzgrs_backend::common::blob::DaBlob;
use network::NetworkAdapter;
use nomos_core::da::BlobId;
use nomos_da_network_service::{backends::libp2p::common::SamplingEvent, NetworkService};
use nomos_da_verifier::{DaVerifierMsg, DaVerifierService};
use nomos_storage::StorageService;
use nomos_tracing::{error_with_id, info_with_id};
use overwatch_rs::{
    services::{
        relay::{OutboundRelay, Relay, RelayMessage},
        state::{NoOperator, NoState},
        ServiceCore, ServiceData, ServiceId,
    },
    DynError, OpaqueServiceStateHandle,
};
use rand::{RngCore, SeedableRng};
use serde::{Deserialize, Serialize};
use services_utils::overwatch::lifecycle;
use storage::DaStorageAdapter;
use tokio::sync::oneshot;
use tokio_stream::StreamExt as _;
use tracing::{error, instrument};

const DA_SAMPLING_TAG: ServiceId = "DA-Sampling";

#[derive(Debug)]
pub enum DaSamplingServiceMsg<BlobId> {
    TriggerSampling {
        blob_id: BlobId,
    },
    GetValidatedBlobs {
        reply_channel: oneshot::Sender<BTreeSet<BlobId>>,
    },
    MarkInBlock {
        blobs_id: Vec<BlobId>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaSamplingServiceSettings<BackendSettings, NetworkSettings, StorageSettings> {
    pub sampling_settings: BackendSettings,
    pub network_adapter_settings: NetworkSettings,
    pub storage_adapter_settings: StorageSettings,
}

impl<B: 'static> RelayMessage for DaSamplingServiceMsg<B> {}

pub struct DaSamplingService<
    Backend,
    DaNetwork,
    SamplingRng,
    DaStorage,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
> where
    SamplingRng: SeedableRng + RngCore,
    Backend: DaSamplingServiceBackend<SamplingRng> + Send,
    Backend::Settings: Clone,
    Backend::Blob: Debug + 'static,
    Backend::BlobId: Debug + 'static,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter,
    DaVerifierNetwork::Settings: Clone,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send,
    DaVerifierBackend::Settings: Clone,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter,
    DaNetwork: NetworkAdapter,
    DaNetwork::Settings: Clone,
    DaStorage: DaStorageAdapter,
    <DaVerifierBackend as nomos_core::da::DaVerifier>::DaBlob: 'static,
{
    network_relay: Relay<NetworkService<DaNetwork::Backend>>,
    storage_relay: Relay<StorageService<DaStorage::Backend>>,
    verifier_relay:
        Relay<DaVerifierService<DaVerifierBackend, DaVerifierNetwork, DaVerifierStorage>>,
    service_state: OpaqueServiceStateHandle<Self>,
    sampler: Backend,
}

impl<
        Backend,
        DaNetwork,
        SamplingRng,
        DaStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
    >
    DaSamplingService<
        Backend,
        DaNetwork,
        SamplingRng,
        DaStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
    >
where
    SamplingRng: SeedableRng + RngCore,
    Backend: DaSamplingServiceBackend<SamplingRng, BlobId = BlobId, Blob = DaBlob> + Send + 'static,
    Backend::Settings: Clone,
    DaNetwork: NetworkAdapter + Send + 'static,
    DaNetwork::Settings: Clone,
    DaStorage: DaStorageAdapter<Blob = DaBlob> + Sync,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter + Send,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend<DaBlob = DaBlob> + Send,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter,
    <DaVerifierNetwork as nomos_da_verifier::network::NetworkAdapter>::Settings: Clone,
{
    #[instrument(skip_all)]
    async fn handle_service_message(
        msg: <Self as ServiceData>::Message,
        network_adapter: &mut DaNetwork,
        sampler: &mut Backend,
    ) {
        match msg {
            DaSamplingServiceMsg::TriggerSampling { blob_id } => {
                if let SamplingState::Init(sampling_subnets) = sampler.init_sampling(blob_id).await
                {
                    info_with_id!(blob_id, "TriggerSampling");
                    if let Err(e) = network_adapter
                        .start_sampling(blob_id, &sampling_subnets)
                        .await
                    {
                        // we can short circuit the failure from beginning
                        sampler.handle_sampling_error(blob_id).await;
                        error_with_id!(blob_id, "Error sampling for BlobId: {blob_id:?}: {e}");
                    }
                }
            }
            DaSamplingServiceMsg::GetValidatedBlobs { reply_channel } => {
                let validated_blobs = sampler.get_validated_blobs().await;
                if let Err(_e) = reply_channel.send(validated_blobs) {
                    error!("Error repliying validated blobs request");
                }
            }
            DaSamplingServiceMsg::MarkInBlock { blobs_id } => {
                sampler.mark_completed(&blobs_id).await;
            }
        }
    }

    #[instrument(skip_all)]
    async fn handle_sampling_message(
        event: SamplingEvent,
        sampler: &mut Backend,
        storage_adapter: &DaStorage,
        verifier_relay: &OutboundRelay<DaVerifierMsg<DaVerifierBackend::DaBlob, ()>>,
    ) {
        match event {
            SamplingEvent::SamplingSuccess { blob_id, blob } => {
                info_with_id!(blob_id, "SamplingSuccess");
                if let Ok(blob) = Self::verify_blob(verifier_relay, *blob).await {
                    sampler.handle_sampling_success(blob_id, blob).await;
                } else {
                    error_with_id!(blob_id, "SamplingError");
                    sampler.handle_sampling_error(blob_id).await;
                }
                return;
            }
            SamplingEvent::SamplingError { error } => {
                if let Some(blob_id) = error.blob_id() {
                    error_with_id!(blob_id, "SamplingError");
                    sampler.handle_sampling_error(*blob_id).await;
                    return;
                }
                error!("Error while sampling: {error}");
            }
            SamplingEvent::SamplingRequest {
                blob_id,
                column_idx,
                response_sender,
            } => {
                info_with_id!(blob_id, "SamplingRequest");
                let maybe_blob = storage_adapter
                    .get_blob(blob_id, column_idx)
                    .await
                    .map_err(|error| {
                        error!("Failed to get blob from storage adapter: {error}");
                        None::<Backend::Blob>
                    })
                    .ok()
                    .flatten();

                if response_sender.send(maybe_blob).await.is_err() {
                    error!("Error sending sampling response");
                }
            }
        }
    }

    async fn verify_blob(
        verifier_relay: &OutboundRelay<DaVerifierMsg<DaBlob, ()>>,
        blob: DaBlob,
    ) -> Result<DaBlob, DynError> {
        let (reply_sender, reply_receiver) = oneshot::channel();
        verifier_relay
            .send(DaVerifierMsg::VerifyBlob {
                blob,
                reply_channel: reply_sender,
            })
            .await
            .expect("Failed to send verify blob message to verifier relay");

        reply_receiver
            .await
            .expect("Failed to receive reply blob message from verifier relay")
    }
}

impl<
        Backend,
        DaNetwork,
        SamplingRng,
        DaStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
    > ServiceData
    for DaSamplingService<
        Backend,
        DaNetwork,
        SamplingRng,
        DaStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
    >
where
    SamplingRng: SeedableRng + RngCore,
    Backend: DaSamplingServiceBackend<SamplingRng> + Send,
    Backend::Settings: Clone,
    Backend::Blob: Debug + 'static,
    Backend::BlobId: Debug + 'static,
    DaNetwork: NetworkAdapter,
    DaNetwork::Settings: Clone,
    DaStorage: DaStorageAdapter,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter,
    DaVerifierNetwork::Settings: Clone,
{
    const SERVICE_ID: ServiceId = DA_SAMPLING_TAG;
    type Settings =
        DaSamplingServiceSettings<Backend::Settings, DaNetwork::Settings, DaStorage::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State, Self::Settings>;
    type Message = DaSamplingServiceMsg<Backend::BlobId>;
}

#[async_trait::async_trait]
impl<
        Backend,
        DaNetwork,
        SamplingRng,
        DaStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
    > ServiceCore
    for DaSamplingService<
        Backend,
        DaNetwork,
        SamplingRng,
        DaStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
    >
where
    SamplingRng: SeedableRng + RngCore,
    Backend: DaSamplingServiceBackend<SamplingRng, BlobId = BlobId, Blob = DaBlob>
        + Send
        + Sync
        + 'static,
    Backend::Settings: Clone + Send + Sync + 'static,
    DaNetwork: NetworkAdapter + Send + Sync + 'static,
    DaNetwork::Settings: Clone + Send + Sync + 'static,
    DaStorage: DaStorageAdapter<Blob = Backend::Blob> + Sync + Send,
    DaVerifierBackend:
        nomos_da_verifier::backend::VerifierBackend<DaBlob = Backend::Blob> + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter,
    DaVerifierNetwork::Settings: Clone,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter + Send + Sync,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        let DaSamplingServiceSettings {
            sampling_settings, ..
        } = service_state.settings_reader.get_updated_settings();

        let network_relay = service_state.overwatch_handle.relay();
        let storage_relay = service_state.overwatch_handle.relay();
        let verifier_relay = service_state.overwatch_handle.relay();
        let rng = SamplingRng::from_entropy();

        Ok(Self {
            network_relay,
            storage_relay,
            verifier_relay,
            service_state,
            sampler: Backend::new(sampling_settings, rng),
        })
    }

    async fn run(self) -> Result<(), DynError> {
        // This service will likely have to be modified later on.
        // Most probably the verifier itself need to be constructed/update for every
        // message with an updated list of the available nodes list, as it needs
        // his own index coming from the position of his bls public key landing
        // in the above-mentioned list.
        let Self {
            network_relay,
            storage_relay,
            verifier_relay,
            mut service_state,
            mut sampler,
        } = self;

        let network_relay = network_relay.connect().await?;
        let mut network_adapter = DaNetwork::new(network_relay).await;

        let mut sampling_message_stream = network_adapter.listen_to_sampling_messages().await?;
        let mut next_prune_tick = sampler.prune_interval();

        let storage_relay = storage_relay.connect().await?;
        let storage_adapter = DaStorage::new(storage_relay).await;

        let verifier_relay = verifier_relay.connect().await?;

        let mut lifecycle_stream = service_state.lifecycle_handle.message_stream();
        #[expect(clippy::redundant_pub_crate)]
        loop {
            tokio::select! {
                Some(service_message) = service_state.inbound_relay.recv() => {
                    Self::handle_service_message(service_message, &mut network_adapter, &mut sampler).await;
                }
                Some(sampling_message) = sampling_message_stream.next() => {
                    Self::handle_sampling_message(sampling_message, &mut sampler, &storage_adapter, &verifier_relay).await;
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self>(&msg) {
                        break;
                    }
                }
                // cleanup not on time samples
                _ = next_prune_tick.tick() => {
                    sampler.prune();
                }

            }
        }

        Ok(())
    }
}
