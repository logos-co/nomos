pub mod backend;
pub mod network;
pub mod storage;
pub mod verifier;

use std::{
    collections::{BTreeSet, HashMap},
    fmt::{Debug, Display},
    marker::PhantomData,
    pin::Pin,
    time::Duration,
};

use backend::{DaSamplingServiceBackend, SamplingState};
use futures::Stream;
use kzgrs_backend::common::share::{DaShare, DaSharesCommitments};
use network::NetworkAdapter;
use nomos_core::da::BlobId;
use nomos_da_network_core::protocols::sampling::errors::SamplingError;
use nomos_da_network_service::{
    backends::libp2p::common::{CommitmentsEvent, SamplingEvent},
    NetworkService,
};
use nomos_storage::StorageService;
use nomos_tracing::{error_with_id, info_with_id};
use overwatch::{
    services::{
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
use verifier::{kzgrs::KzgrsDaVerifier, VerifierBackend};

pub type DaSamplingService<SamplingBackend, SamplingNetwork, SamplingStorage, RuntimeServiceId> =
    GenericDaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        KzgrsDaVerifier,
        RuntimeServiceId,
    >;

pub type PendingCommitmentRequests =
    HashMap<BlobId, Vec<oneshot::Sender<Option<DaSharesCommitments>>>>;

#[derive(Debug)]
pub enum DaSamplingServiceMsg<BlobId> {
    TriggerSampling {
        blob_id: BlobId,
    },
    GetCommitments {
        blob_id: BlobId,
        response_sender: oneshot::Sender<Option<DaSharesCommitments>>,
    },
    GetValidatedBlobs {
        reply_channel: oneshot::Sender<BTreeSet<BlobId>>,
    },
    MarkInBlock {
        blobs_id: Vec<BlobId>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaSamplingServiceSettings<BackendSettings, ShareVerifierSettings> {
    pub sampling_settings: BackendSettings,
    pub share_verifier_settings: ShareVerifierSettings,
    pub commitments_wait_duration: Duration,
}

pub struct GenericDaSamplingService<
    SamplingBackend,
    SamplingNetwork,
    SamplingStorage,
    ShareVerifier,
    RuntimeServiceId,
> where
    SamplingBackend: DaSamplingServiceBackend,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId>,
    ShareVerifier: VerifierBackend,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        ShareVerifier,
    )>,
}

impl<SamplingBackend, SamplingNetwork, SamplingStorage, ShareVerifier, RuntimeServiceId>
    GenericDaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        ShareVerifier,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId>,
    ShareVerifier: VerifierBackend,
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

impl<SamplingBackend, SamplingNetwork, SamplingStorage, ShareVerifier, RuntimeServiceId>
    GenericDaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        ShareVerifier,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend<
            BlobId = BlobId,
            Share = DaShare,
            SharesCommitments = DaSharesCommitments,
        > + Send,
    SamplingBackend::Settings: Clone,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId, Share = DaShare> + Send + Sync,
    ShareVerifier: VerifierBackend<DaShare = DaShare> + Send + Sync,
{
    #[instrument(skip_all)]
    async fn handle_service_message(
        msg: <Self as ServiceData>::Message,
        network_adapter: &mut SamplingNetwork,
        storage_adapter: &SamplingStorage,
        sampler: &mut SamplingBackend,
        commitments_wait_duration: Duration,
    ) {
        match msg {
            DaSamplingServiceMsg::TriggerSampling { blob_id } => {
                if matches!(sampler.init_sampling(blob_id).await, SamplingState::Init) {
                    info_with_id!(blob_id, "InitSampling");
                    if let Some(commitments) = Self::request_commitments(
                        storage_adapter,
                        network_adapter,
                        commitments_wait_duration,
                        blob_id,
                    )
                    .await
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
            DaSamplingServiceMsg::GetCommitments {
                blob_id,
                response_sender,
            } => {
                let commitments = Self::request_commitments(
                    storage_adapter,
                    network_adapter,
                    commitments_wait_duration,
                    blob_id,
                )
                .await;
                if let Err(err) = response_sender.send(commitments) {
                    error!("Error replying share commitments request: {err:?}");
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
        verifier: &ShareVerifier,
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
                if verifier.verify(&commitments, &light_share).is_ok() {
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

    #[expect(
        clippy::cognitive_complexity,
        reason = "nested error check when writing into response sender"
    )]
    async fn handle_commitments_message(
        storage_adapter: &SamplingStorage,
        commitments_message: CommitmentsEvent,
    ) {
        match commitments_message {
            CommitmentsEvent::CommitmentsSuccess { .. } => {
                // Handled on demand with `wait_commitments`, this stream
                // handler ignores such messages.
            }
            CommitmentsEvent::CommitmentsRequest {
                blob_id,
                response_sender,
            } => {
                if let Ok(commitments) = storage_adapter.get_commitments(blob_id).await {
                    if let Err(err) = response_sender.send(commitments).await {
                        tracing::error!("Couldn't send commitments response: {err:?}");
                    }
                }
            }
            CommitmentsEvent::CommitmentsError { error } => match error.blob_id() {
                Some(blob_id) => {
                    error_with_id!(blob_id, "Commitments response error: {error}");
                }
                None => {
                    tracing::error!("Commitments response error: {error}");
                }
            },
        }
    }

    async fn request_commitments(
        storage_adapter: &SamplingStorage,
        network_adapter: &SamplingNetwork,
        wait_duration: Duration,
        blob_id: SamplingBackend::BlobId,
    ) -> Option<DaSharesCommitments> {
        // First try to get from storage which most of the time should be the case
        if let Ok(Some(commitments)) = storage_adapter.get_commitments(blob_id).await {
            return Some(commitments);
        }

        // Fall back to API request
        let (sender, receiver) = oneshot::channel();
        let Ok(stream) = network_adapter.listen_to_commitments_messages().await else {
            tracing::error!("Error subscribing to commitments stream");
            return None;
        };

        network_adapter.request_commitments(blob_id).await.ok()?;
        tokio::spawn(wait_commitments(stream, wait_duration, sender, blob_id));

        receiver.await.ok().flatten()
    }
}

impl<SamplingBackend, SamplingNetwork, SamplingStorage, ShareVerifier, RuntimeServiceId> ServiceData
    for GenericDaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        ShareVerifier,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId>,
    ShareVerifier: VerifierBackend,
{
    type Settings = DaSamplingServiceSettings<SamplingBackend::Settings, ShareVerifier::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaSamplingServiceMsg<SamplingBackend::BlobId>;
}

#[async_trait::async_trait]
impl<SamplingBackend, SamplingNetwork, SamplingStorage, ShareVerifier, RuntimeServiceId>
    ServiceCore<RuntimeServiceId>
    for GenericDaSamplingService<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        ShareVerifier,
        RuntimeServiceId,
    >
where
    SamplingBackend: DaSamplingServiceBackend<
            BlobId = BlobId,
            Share = DaShare,
            SharesCommitments = DaSharesCommitments,
        > + Send,
    SamplingBackend::Settings: Clone + Send + Sync,
    SamplingNetwork: NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    SamplingNetwork::Settings: Send + Sync,
    SamplingNetwork::Membership: MembershipHandler + Clone + 'static,
    SamplingStorage: DaStorageAdapter<RuntimeServiceId, Share = DaShare> + Send + Sync,
    ShareVerifier: VerifierBackend<DaShare = DaShare> + Send + Sync,
    ShareVerifier::Settings: Clone + Send + Sync,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<
            NetworkService<
                SamplingNetwork::Backend,
                SamplingNetwork::Membership,
                SamplingNetwork::MembershipAdapter,
                SamplingNetwork::Storage,
                SamplingNetwork::ApiAdapter,
                RuntimeServiceId,
            >,
        > + AsServiceId<StorageService<SamplingStorage::Backend, RuntimeServiceId>>
        + Debug
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
        let DaSamplingServiceSettings {
            sampling_settings,
            share_verifier_settings,
            commitments_wait_duration,
        } = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let network_relay = service_resources_handle
            .overwatch_handle
            .relay::<NetworkService<_, _, _, _, _, _>>()
            .await?;
        let mut network_adapter = SamplingNetwork::new(network_relay).await;
        let mut sampling_message_stream = network_adapter.listen_to_sampling_messages().await?;
        let mut commitments_message_stream =
            network_adapter.listen_to_commitments_messages().await?;

        let storage_relay = service_resources_handle
            .overwatch_handle
            .relay::<StorageService<_, _>>()
            .await?;
        let storage_adapter = SamplingStorage::new(storage_relay).await;

        let mut sampler = SamplingBackend::new(sampling_settings);
        let share_verifier = ShareVerifier::new(share_verifier_settings);
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
            StorageService<_, _>
        )
        .await?;

        loop {
            tokio::select! {
                Some(service_message) = service_resources_handle.inbound_relay.recv() => {
                    Self::handle_service_message(
                        service_message,
                        &mut network_adapter,
                        &storage_adapter,
                        &mut sampler,
                        commitments_wait_duration,
                    ).await;
                }
                Some(sampling_message) = sampling_message_stream.next() => {
                    Self::handle_sampling_message(
                        sampling_message,
                        &mut sampler,
                        &storage_adapter,
                        &share_verifier
                    ).await;
                }
                Some(commitments_message) = commitments_message_stream.next() => {
                    Self::handle_commitments_message(
                        &storage_adapter,
                        commitments_message
                    ).await;
                }
                // cleanup not on time samples
                _ = next_prune_tick.tick() => {
                    sampler.prune();
                }
            }
        }
    }
}

async fn wait_commitments(
    mut stream: Pin<Box<dyn Stream<Item = CommitmentsEvent> + Send>>,
    wait_duration: Duration,
    sender: oneshot::Sender<Option<DaSharesCommitments>>,
    requested_blob_id: BlobId,
) {
    let _ = tokio::time::timeout(wait_duration, async {
        while let Some(message) = stream.next().await {
            if let CommitmentsEvent::CommitmentsSuccess {
                blob_id,
                commitments,
            } = message
            {
                if blob_id == requested_blob_id {
                    let _ = sender.send(Some(*commitments));
                    return;
                }
            }
        }
        let _ = sender.send(None);
    })
    .await;
}
