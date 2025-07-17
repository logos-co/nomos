pub mod backend;
pub mod network;
pub mod storage;

use std::{
    collections::BTreeSet,
    fmt::{Debug, Display},
    marker::PhantomData,
    sync::Arc,
    time::Duration,
};

use backend::{DaSamplingServiceBackend, SamplingState};
use kzgrs_backend::common::share::{DaLightShare, DaShare, DaSharesCommitments};
use network::NetworkAdapter;
use nomos_core::da::{blob::Share, BlobId, DaVerifier};
use nomos_da_network_core::protocols::sampling::errors::SamplingError;
use nomos_da_network_service::{
    backends::libp2p::common::SamplingEvent, membership::MembershipAdapter,
    storage::MembershipStorageAdapter, NetworkService,
};
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
    DynError, OpaqueServiceResourcesHandle,
};
use serde::{Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use storage::DaStorageAdapter;
use subnetworks_assignations::MembershipHandler;
use tokio::sync::oneshot;
use tokio_stream::StreamExt as _;
use tracing::{error, instrument};

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
pub struct DaSamplingServiceSettings<BackendSettings> {
    pub sampling_settings: BackendSettings,
}

pub struct DaSamplingService<
    SamplingBackend,
    SamplingNetwork,
    SamplingStorage,
    VerifierBackend,
    VerifierNetwork,
    VerifierStorage,
    RuntimeServiceId,
> where
    SamplingBackend: DaSamplingServiceBackend,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId>,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
    )>,
}

impl<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        RuntimeServiceId,
    >
    DaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId>,
{
    #[must_use]
    pub const fn new(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    ) -> Self {
        Self {
            service_resources_handle,
            _phantom: PhantomData,
        }
    }
}

impl<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        RuntimeServiceId,
    >
    DaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend<
            BlobId = BlobId,
            Share = DaShare,
            SharesCommitments = DaSharesCommitments,
        > + Send,
    SamplingBackend::Settings: Clone,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId, Share = DaShare> + Send + Sync,
    VerifierBackend: VerifierBackendTrait<DaShare = DaShare>,
{
    #[instrument(skip_all)]
    async fn handle_service_message(
        msg: <Self as ServiceData>::Message,
        network_adapter: &mut SamplingNetwork,
        storage_adapter: &SamplingStorage,
        sampler: &mut SamplingBackend,
    ) {
        match msg {
            DaSamplingServiceMsg::TriggerSampling { blob_id } => {
                if matches!(sampler.init_sampling(blob_id).await, SamplingState::Init) {
                    info_with_id!(blob_id, "InitSampling");
                    if let Some(commitments) =
                        Self::request_commitments(storage_adapter, network_adapter, blob_id).await
                    {
                        info_with_id!(blob_id, "Got commitments");
                        sampler.add_commitments(&blob_id, commitments);
                    } else {
                        error_with_id!(blob_id, "Error getting commitments");
                        sampler.handle_sampling_error(blob_id).await;
                        return;
                    }

                    if let Err(e) = network_adapter.start_sampling(blob_id).await {
                        // we can short circuit the failure from the beginning
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
        network_adapter: &SamplingNetwork,
        blob_id: SamplingBackend::BlobId,
    ) -> Option<DaSharesCommitments> {
        // First try to get from storage which most of the time should be the case
        if let Ok(Some(commitments)) = storage_adapter.get_commitments(blob_id).await {
            return Some(commitments);
        }

        // Fall back to API request
        network_adapter
            .get_commitments(blob_id)
            .await
            .ok()
            .flatten()
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
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        RuntimeServiceId,
    > ServiceData
    for DaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId>,
{
    type Settings = DaSamplingServiceSettings<SamplingBackend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaSamplingServiceMsg<SamplingBackend::BlobId>;
}

#[async_trait::async_trait]
impl<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        RuntimeServiceId,
    > ServiceCore<RuntimeServiceId>
    for DaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        VerifierBackend,
        VerifierNetwork,
        VerifierStorage,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend<
            BlobId = BlobId,
            Share = DaShare,
            SharesCommitments = DaSharesCommitments,
        > + Send,
    SamplingBackend::Settings: Clone + Send + Sync,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingNetwork::Settings: Send + Sync,
    SamplingNetwork::Membership: MembershipHandler + Clone + 'static,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId, Share = DaShare> + Send + Sync,
    VerifierBackend:
        nomos_da_verifier::backend::VerifierBackend<DaShare = SamplingBackend::Share> + Send,
    VerifierBackend::Settings: Clone,
    VerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send,
    VerifierNetwork::Settings: Clone,
    VerifierNetwork::Storage: MembershipStorageAdapter<
            <SamplingNetwork::Membership as MembershipHandler>::Id,
            <SamplingNetwork::Membership as MembershipHandler>::NetworkId,
        > + Send
        + Sync
        + 'static,
    VerifierNetwork::MembershipAdapter: MembershipAdapter,
    VerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<
            NetworkService<
                SamplingNetwork::Backend,
                SamplingNetwork::Membership,
                VerifierNetwork::MembershipAdapter,
                VerifierNetwork::Storage,
                VerifierNetwork::ApiAdapter,
                RuntimeServiceId,
            >,
        > + AsServiceId<StorageService<SamplingStorage::Backend, RuntimeServiceId>>
        + AsServiceId<
            DaVerifierService<VerifierBackend, VerifierNetwork, VerifierStorage, RuntimeServiceId>,
        > + Debug
        + Display
        + Sync
        + Send
        + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self::new(service_resources_handle))
    }

    async fn run(mut self) -> Result<(), DynError> {
        let Self {
            mut service_resources_handle,
            ..
        } = self;
        let DaSamplingServiceSettings { sampling_settings } = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let network_relay = service_resources_handle
            .overwatch_handle
            .relay::<NetworkService<_, _, _, _, _, _>>()
            .await?;
        let mut network_adapter = SamplingNetwork::new(network_relay).await;
        let mut sampling_message_stream = network_adapter.listen_to_sampling_messages().await?;

        let storage_relay = service_resources_handle
            .overwatch_handle
            .relay::<StorageService<_, _>>()
            .await?;
        let storage_adapter = SamplingStorage::new(storage_relay).await;

        let verifier_relay = service_resources_handle
            .overwatch_handle
            .relay::<DaVerifierService<_, _, _, _>>()
            .await?;

        let mut sampler = SamplingBackend::new(sampling_settings);
        let mut next_prune_tick = sampler.prune_interval();

        service_resources_handle.status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _, _, _,_, _>,
            StorageService<_, _>,
            DaVerifierService<_, _, _, _>
        )
        .await?;

        loop {
            tokio::select! {
                Some(service_message) = service_resources_handle.inbound_relay.recv() => {
                    Self::handle_service_message(service_message, &mut network_adapter,  &storage_adapter,  &mut sampler).await;
                }
                Some(sampling_message) = sampling_message_stream.next() => {
                    Self::handle_sampling_message(sampling_message, &mut sampler, &storage_adapter, &verifier_relay).await;
                }
                // cleanup not on time samples
                _ = next_prune_tick.tick() => {
                    sampler.prune();
                }

            }
        }
    }
}
