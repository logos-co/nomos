pub mod backend;
pub mod network;
pub mod storage;

use std::{
    error::Error,
    fmt::{Debug, Display, Formatter},
    sync::Arc,
    time::Duration,
};

use backend::VerifierBackend;
use network::NetworkAdapter;
use nomos_core::da::blob::Share;
use nomos_da_network_service::NetworkService;
use nomos_storage::StorageService;
use nomos_tracing::info_with_id;
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
use tokio::sync::oneshot::Sender;
use tokio_stream::StreamExt as _;
use tracing::{error, instrument};

pub enum DaVerifierMsg<Commitments, LightShare, Share, Answer> {
    AddShare {
        share: Share,
        reply_channel: Sender<Option<Answer>>,
    },
    VerifyShare {
        commitments: Arc<Commitments>,
        light_share: Box<LightShare>,
        reply_channel: Sender<Result<(), DynError>>,
    },
}

impl<C: 'static, L: 'static, B: 'static, A: 'static> Debug for DaVerifierMsg<C, L, B, A> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddShare { .. } => {
                write!(f, "DaVerifierMsg::AddShare")
            }
            Self::VerifyShare { .. } => {
                write!(f, "DaVerifierMsg::VerifyShare")
            }
        }
    }
}

pub struct DaVerifierService<Backend, N, S, RuntimeServiceId>
where
    Backend: VerifierBackend,
    Backend::Settings: Clone,
    Backend::DaShare: 'static,
    N: NetworkAdapter<RuntimeServiceId>,
    N::Settings: Clone,
    S: DaStorageAdapter<RuntimeServiceId>,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    verifier: Backend,
}

impl<Backend, N, S, RuntimeServiceId> DaVerifierService<Backend, N, S, RuntimeServiceId>
where
    Backend: VerifierBackend + Send + Sync + 'static,
    Backend::DaShare: Debug + Send,
    Backend::Error: Error + Send + Sync,
    Backend::Settings: Clone,
    <Backend::DaShare as Share>::BlobId: AsRef<[u8]>,
    N: NetworkAdapter<RuntimeServiceId, Share = Backend::DaShare> + Send + 'static,
    N::Settings: Clone,
    S: DaStorageAdapter<RuntimeServiceId, Share = Backend::DaShare> + Send + Sync + 'static,
{
    #[instrument(skip_all)]
    async fn handle_new_share(
        verifier: &Backend,
        storage_adapter: &S,
        share: Backend::DaShare,
    ) -> Result<(), DynError> {
        if storage_adapter
            .get_share(share.blob_id(), share.share_idx())
            .await?
            .is_some()
        {
            info_with_id!(share.blob_id().as_ref(), "VerifierShareExists");
        } else {
            info_with_id!(share.blob_id().as_ref(), "VerifierAddShare");
            let (blob_id, share_idx) = (share.blob_id(), share.share_idx());
            let (light_share, commitments) = share.into_share_and_commitments();
            verifier.verify(&commitments, &light_share)?;
            storage_adapter
                .add_share(blob_id, share_idx, commitments, light_share)
                .await?;
        }
        Ok(())
    }
}

impl<Backend, N, S, RuntimeServiceId> ServiceData
    for DaVerifierService<Backend, N, S, RuntimeServiceId>
where
    Backend: VerifierBackend,
    Backend::Settings: Clone,
    N: NetworkAdapter<RuntimeServiceId>,
    N::Settings: Clone,
    S: DaStorageAdapter<RuntimeServiceId>,
    S::Settings: Clone,
{
    type Settings = DaVerifierServiceSettings<Backend::Settings, N::Settings, S::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaVerifierMsg<
        <Backend::DaShare as Share>::SharesCommitments,
        <Backend::DaShare as Share>::LightShare,
        Backend::DaShare,
        (),
    >;
}

#[async_trait::async_trait]
impl<Backend, N, S, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for DaVerifierService<Backend, N, S, RuntimeServiceId>
where
    Backend: VerifierBackend + Send + Sync + 'static,
    Backend::Settings: Clone + Send + Sync + 'static,
    Backend::DaShare: Debug + Send + Sync + 'static,
    Backend::Error: Error + Send + Sync + 'static,
    <Backend::DaShare as Share>::BlobId: AsRef<[u8]> + Debug + Send + Sync + 'static,
    <Backend::DaShare as Share>::LightShare: Debug + Send + Sync + 'static,
    <Backend::DaShare as Share>::SharesCommitments: Debug + Send + Sync + 'static,
    N: NetworkAdapter<RuntimeServiceId, Share = Backend::DaShare> + Send + Sync + 'static,
    N::Membership: MembershipHandler + Clone,
    N::Settings: Clone + Send + Sync + 'static,
    S: DaStorageAdapter<RuntimeServiceId, Share = Backend::DaShare> + Send + Sync + 'static,
    S::Settings: Clone + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Display
        + Sync
        + Send
        + 'static
        + AsServiceId<Self>
        + AsServiceId<NetworkService<N::Backend, N::Membership, RuntimeServiceId>>
        + AsServiceId<StorageService<S::Backend, RuntimeServiceId>>,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        let DaVerifierServiceSettings {
            verifier_settings, ..
        } = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        Ok(Self {
            service_resources_handle,
            verifier: Backend::new(verifier_settings),
        })
    }

    async fn run(self) -> Result<(), DynError> {
        // This service will likely have to be modified later on.
        // Most probably the verifier itself need to be constructed/update for every
        // message with an updated list of the available nodes list, as it needs
        // his own index coming from the position of his bls public key landing
        // in the above-mentioned list.
        let Self {
            mut service_resources_handle,
            verifier,
        } = self;

        let DaVerifierServiceSettings {
            network_adapter_settings,
            ..
        } = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let network_relay = service_resources_handle
            .overwatch_handle
            .relay::<NetworkService<_, _, _>>()
            .await?;
        let network_adapter = N::new(network_adapter_settings, network_relay).await;
        let mut share_stream = network_adapter.share_stream().await;

        let storage_relay = service_resources_handle
            .overwatch_handle
            .relay::<StorageService<_, _>>()
            .await?;
        let storage_adapter = S::new(storage_relay).await;

        service_resources_handle.status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _, _>,
            StorageService<_, _>
        )
        .await?;

        loop {
            tokio::select! {
                Some(share) = share_stream.next() => {
                    let blob_id = share.blob_id();
                    if let Err(err) =  Self::handle_new_share(&verifier,&storage_adapter, share).await {
                        error!("Error handling blob {blob_id:?} due to {err:?}");
                    }
                }
                Some(msg) = service_resources_handle.inbound_relay.recv() => {
                    match msg {
                        DaVerifierMsg::AddShare { share, reply_channel } => {
                            let blob_id = share.blob_id();
                            match Self::handle_new_share(&verifier, &storage_adapter, share).await {
                                Ok(attestation) => {
                                    if let Err(err) = reply_channel.send(Some(attestation)) {
                                        error!("Error replying attestation {err:?}");
                                    }
                                },
                                Err(err) => {
                                    error!("Error handling blob {blob_id:?} due to {err:?}");
                                    if let Err(err) = reply_channel.send(None) {
                                        error!("Error replying attestation {err:?}");
                                    }
                                },
                            };
                        },
                        DaVerifierMsg::VerifyShare {commitments,  light_share, reply_channel } => {
                            match verifier.verify(&commitments, &light_share) {
                                Ok(()) => {
                                    if let Err(err) = reply_channel.send(Ok(())) {
                                        error!("Error replying verification {err:?}");
                                    }
                                },
                                Err(err) => {
                                    error!("Error verifying blob due to {err:?}");
                                    if let Err(err) = reply_channel.send(Err(err.into())) {
                                        error!("Error replying verification {err:?}");
                                    }
                                },
                            }
                        },

                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaVerifierServiceSettings<BackendSettings, NetworkSettings, StorageSettings> {
    pub verifier_settings: BackendSettings,
    pub network_adapter_settings: NetworkSettings,
    pub storage_adapter_settings: StorageSettings,
}
