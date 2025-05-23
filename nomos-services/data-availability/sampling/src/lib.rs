pub mod api;
pub mod backend;
pub mod network;
pub mod storage;

use std::{
    collections::BTreeSet,
    fmt::{Debug, Display},
    marker::PhantomData,
    sync::Arc,
};

use backend::{DaSamplingServiceBackend, SamplingState};
use kzgrs_backend::common::share::{DaLightShare, DaShare, DaSharesCommitments};
use network::NetworkAdapter;
use nomos_core::da::{blob::Share, BlobId, DaVerifier};
use nomos_da_network_core::protocols::sampling::behaviour::SamplingError;
use nomos_da_network_service::{backends::libp2p::common::SamplingEvent, NetworkService};
use nomos_da_verifier::{
    backend::VerifierBackend as VerifierBackendTrait, DaVerifierMsg, DaVerifierService,
};
use nomos_storage::StorageService;
use nomos_tracing::{error_with_id, info_with_id};
use overwatch::{
    services::{
        relay::OutboundRelay,
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use services_utils::overwatch::lifecycle;
use storage::DaStorageAdapter;
use tokio::sync::oneshot;
use tokio_stream::StreamExt as _;
use tracing::{error, instrument};

use crate::api::ApiAdapter as ApiAdapterTrait;

type VerifierRelay<DaVerifierBackend> = OutboundRelay<
    DaVerifierMsg<
        <<DaVerifierBackend as DaVerifier>::DaShare as Share>::SharesCommitments,
        <<DaVerifierBackend as DaVerifier>::DaShare as Share>::LightShare,
        <DaVerifierBackend as DaVerifier>::DaShare,
        (),
    >,
>;

type VerifierMessage = DaVerifierMsg<DaSharesCommitments, DaLightShare, DaShare, ()>;

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
pub struct DaSamplingServiceSettings<BackendSettings, ApiAdapterSettings> {
    pub sampling_settings: BackendSettings,
    pub api_adapter_settings: ApiAdapterSettings,
}

pub struct DaSamplingService<
    SamplingBackend,
    SamplingNetwork,
    SamplingRng,
    SamplingStorage,
    VerifierBackend,
    VerifierNetwork,
    VerifierStorage,
    ApiAdapter,
    RuntimeServiceId,
> where
    SamplingBackend: DaSamplingServiceBackend<SamplingRng>,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId>,
    SamplingRng: Rng,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId>,
    ApiAdapter: ApiAdapterTrait,
{
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
    #[expect(clippy::type_complexity, reason = "No other way around this for now.")]
    _phantom: PhantomData<(
        SamplingBackend,
        SamplingNetwork,
        SamplingRng,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        ApiAdapter,
    )>,
}

impl<
        SamplingBackend,
        SamplingNetwork,
        SamplingRng,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        ApiAdapter,
        RuntimeServiceId,
    >
    DaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingRng,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend<SamplingRng>,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId>,
    SamplingRng: Rng,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId>,
    ApiAdapter: ApiAdapterTrait,
{
    #[must_use]
    pub const fn new(service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>) -> Self {
        Self {
            service_state,
            _phantom: PhantomData,
        }
    }
}

impl<
        SamplingBackend,
        SamplingNetwork,
        SamplingRng,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        ApiAdapter,
        RuntimeServiceId,
    >
    DaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingRng,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend<
            SamplingRng,
            BlobId = BlobId,
            Share = DaShare,
            SharesCommitments = DaSharesCommitments,
        > + Send,
    SamplingBackend::Settings: Clone,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId> + Send,
    SamplingRng: Rng + SeedableRng,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId, Share = DaShare> + Send + Sync,
    VerifierBackend: VerifierBackendTrait<DaShare = DaShare>,
    ApiAdapter: ApiAdapterTrait<
            Share = SamplingBackend::Share,
            BlobId = SamplingBackend::BlobId,
            Commitments = SamplingBackend::SharesCommitments,
        > + Sync,
    ApiAdapter::Settings: Clone + Send,
{
    #[instrument(skip_all)]
    async fn handle_service_message(
        msg: <Self as ServiceData>::Message,
        network_adapter: &mut SamplingNetwork,
        storage_adapter: &SamplingStorage,
        api_adapter: &ApiAdapter,
        sampler: &mut SamplingBackend,
    ) {
        match msg {
            DaSamplingServiceMsg::TriggerSampling { blob_id } => {
                if let SamplingState::Init(sampling_subnets) = sampler.init_sampling(blob_id).await
                {
                    info_with_id!(blob_id, "InitSampling");
                    if let Some(commitments) =
                        Self::request_commitments(storage_adapter, api_adapter, blob_id).await
                    {
                        info_with_id!(blob_id, "Got commitments");
                        sampler.add_commitments(&blob_id, commitments);
                    } else {
                        error_with_id!(blob_id, "Error getting commitments");
                        sampler.handle_sampling_error(blob_id).await;
                        return;
                    }

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
        sampler: &mut SamplingBackend,
        storage_adapter: &SamplingStorage,
        verifier_relay: &VerifierRelay<VerifierBackend>,
    ) {
        match event {
            SamplingEvent::SamplingSuccess {
                blob_id,
                light_share,
            } => {
                info_with_id!(blob_id, "SamplingSuccess");
                let Some(commitments) = sampler.get_commitments(&blob_id) else {
                    error_with_id!(blob_id, "Error getting commitments for blob");
                    sampler.handle_sampling_error(blob_id).await;
                    return;
                };
                if Self::verify_blob(verifier_relay, commitments, light_share.clone())
                    .await
                    .is_ok()
                {
                    sampler
                        .handle_sampling_success(blob_id, light_share.share_idx)
                        .await;
                } else {
                    error_with_id!(blob_id, "SamplingError");
                    sampler.handle_sampling_error(blob_id).await;
                }
                return;
            }
            SamplingEvent::SamplingError { error } => {
                if let Some(blob_id) = error.blob_id() {
                    error_with_id!(blob_id, "SamplingError");
                    if let SamplingError::BlobNotFound { .. } = error {
                        sampler.handle_sampling_error(*blob_id).await;
                        return;
                    }
                }
                error!("Error while sampling: {error}");
            }
            SamplingEvent::SamplingRequest {
                blob_id,
                share_idx,
                response_sender,
            } => {
                info_with_id!(blob_id, "SamplingRequest");
                let maybe_share = storage_adapter
                    .get_light_share(blob_id, share_idx.to_be_bytes())
                    .await
                    .map_err(|error| {
                        error!("Failed to get share from storage adapter: {error}");
                        None::<SamplingBackend::Share>
                    })
                    .ok()
                    .flatten();

                if response_sender.send(maybe_share).await.is_err() {
                    error!("Error sending sampling response");
                }
            }
        }
    }

    async fn request_commitments(
        storage_adapter: &SamplingStorage,
        api_request: &ApiAdapter,
        blob_id: SamplingBackend::BlobId,
    ) -> Option<DaSharesCommitments> {
        // First try to get from storage which most of the time should be the case
        if let Ok(Some(commitments)) = storage_adapter.get_commitments(blob_id).await {
            return Some(commitments);
        }

        // Fall back to API request
        let (reply_sender, reply_channel) = oneshot::channel();
        if let Err(e) = api_request.request_commitments(blob_id, reply_sender).await {
            error!("Error sending request to API backend: {e}");
        }

        reply_channel.await.ok().flatten()
    }

    async fn verify_blob(
        verifier_relay: &OutboundRelay<VerifierMessage>,
        commitments: Arc<DaSharesCommitments>,
        light_share: Box<DaLightShare>,
    ) -> Result<(), DynError> {
        let (reply_sender, reply_channel) = oneshot::channel();
        verifier_relay
            .send(DaVerifierMsg::VerifyShare {
                commitments,
                light_share,
                reply_channel: reply_sender,
            })
            .await
            .expect("Failed to send verify blob message to verifier relay");

        reply_channel
            .await
            .expect("Failed to receive reply blob message from verifier relay")
    }
}

impl<
        SamplingBackend,
        SamplingNetwork,
        SamplingRng,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        ApiAdapter,
        RuntimeServiceId,
    > ServiceData
    for DaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingRng,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend<SamplingRng>,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId>,
    SamplingRng: Rng,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId>,
    ApiAdapter: ApiAdapterTrait,
{
    type Settings = DaSamplingServiceSettings<SamplingBackend::Settings, ApiAdapter::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaSamplingServiceMsg<SamplingBackend::BlobId>;
}

#[async_trait::async_trait]
impl<
        SamplingBackend,
        SamplingNetwork,
        SamplingRng,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        ApiAdapter,
        RuntimeServiceId,
    > ServiceCore<RuntimeServiceId>
    for DaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingRng,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend<
            SamplingRng,
            BlobId = BlobId,
            Share = DaShare,
            SharesCommitments = DaSharesCommitments,
        > + Send,
    SamplingBackend::Settings: Clone + Send + Sync,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId> + Send,
    SamplingNetwork::Settings: Send + Sync,
    SamplingRng: Rng + SeedableRng + Send,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId, Share = DaShare> + Send + Sync,
    VerifierBackend:
        nomos_da_verifier::backend::VerifierBackend<DaShare = SamplingBackend::Share> + Send,
    VerifierBackend::Settings: Clone,
    VerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send,
    VerifierNetwork::Settings: Clone,
    VerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send,
    ApiAdapter: ApiAdapterTrait<
            Share = SamplingBackend::Share,
            BlobId = SamplingBackend::BlobId,
            Commitments = SamplingBackend::SharesCommitments,
        > + Send
        + Sync,
    ApiAdapter::Settings: Clone + Send + Sync,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<SamplingNetwork::Backend, RuntimeServiceId>>
        + AsServiceId<StorageService<SamplingStorage::Backend, RuntimeServiceId>>
        + AsServiceId<
            DaVerifierService<VerifierBackend, VerifierNetwork, VerifierStorage, RuntimeServiceId>,
        > + Debug
        + Display
        + Sync
        + Send
        + 'static,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self::new(service_state))
    }

    async fn run(mut self) -> Result<(), DynError> {
        let Self {
            mut service_state, ..
        } = self;
        let DaSamplingServiceSettings {
            sampling_settings,
            api_adapter_settings,
        } = service_state.settings_reader.get_updated_settings();

        let network_relay = service_state
            .overwatch_handle
            .relay::<NetworkService<_, _>>()
            .await?;
        let mut network_adapter = SamplingNetwork::new(network_relay).await;
        let mut sampling_message_stream = network_adapter.listen_to_sampling_messages().await?;

        let storage_relay = service_state
            .overwatch_handle
            .relay::<StorageService<_, _>>()
            .await?;
        let storage_adapter = SamplingStorage::new(storage_relay).await;

        let verifier_relay = service_state
            .overwatch_handle
            .relay::<DaVerifierService<_, _, _, _>>()
            .await?;

        let api_adapter = ApiAdapter::new(api_adapter_settings);

        let rng = SamplingRng::from_entropy();
        let mut sampler = SamplingBackend::new(sampling_settings, rng);
        let mut next_prune_tick = sampler.prune_interval();

        let mut lifecycle_stream = service_state.lifecycle_handle.message_stream();
        loop {
            tokio::select! {
                Some(service_message) = service_state.inbound_relay.recv() => {
                    Self::handle_service_message(service_message, &mut network_adapter,  &storage_adapter, &api_adapter, &mut sampler).await;
                }
                Some(sampling_message) = sampling_message_stream.next() => {
                    Self::handle_sampling_message(sampling_message, &mut sampler, &storage_adapter, &verifier_relay).await;
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
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
