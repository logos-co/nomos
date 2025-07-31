pub mod backend;
pub mod network;
pub mod storage;

use std::{
    error::Error,
    fmt::{Debug, Display, Formatter},
    sync::Arc,
    time::Duration,
};

use backend::{tx::mock::MockTxVerifier, TxVerifierBackend, VerifierBackend};
use network::NetworkAdapter;
use nomos_core::da::blob::Share;
use nomos_da_network_service::{
    membership::MembershipAdapter, storage::MembershipStorageAdapter, NetworkService,
};
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

pub type DaVerifierService<ShareVerifier, Network, Storage, RuntimeServiceId> =
    GenericDaVerifierService<ShareVerifier, MockTxVerifier, Network, Storage, RuntimeServiceId>;

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

pub struct GenericDaVerifierService<ShareVerifier, TxVerifier, Network, Storage, RuntimeServiceId>
where
    ShareVerifier: VerifierBackend,
    ShareVerifier::Settings: Clone,
    ShareVerifier::DaShare: 'static,
    TxVerifier: TxVerifierBackend,
    TxVerifier::Settings: Clone,
    Network: NetworkAdapter<RuntimeServiceId>,
    Network::Settings: Clone,
    Storage: DaStorageAdapter<RuntimeServiceId>,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    share_verifier: ShareVerifier,
    tx_verifier: TxVerifier,
}

impl<ShareVerifier, TxVerifier, Network, Storage, RuntimeServiceId>
    GenericDaVerifierService<ShareVerifier, TxVerifier, Network, Storage, RuntimeServiceId>
where
    ShareVerifier: VerifierBackend + Send + Sync + 'static,
    ShareVerifier::DaShare: Debug + Send,
    ShareVerifier::Error: Error + Send + Sync,
    ShareVerifier::Settings: Clone,
    <ShareVerifier::DaShare as Share>::BlobId: AsRef<[u8]>,
    TxVerifier: TxVerifierBackend,
    TxVerifier::Settings: Clone,
    Network: NetworkAdapter<RuntimeServiceId, Share = ShareVerifier::DaShare, Tx = TxVerifier::Tx>
        + Send
        + 'static,
    Network::Settings: Clone,
    Storage:
        DaStorageAdapter<RuntimeServiceId, Share = ShareVerifier::DaShare> + Send + Sync + 'static,
{
    #[instrument(skip_all)]
    async fn handle_new_share(
        verifier: &ShareVerifier,
        storage_adapter: &Storage,
        share: ShareVerifier::DaShare,
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

    async fn handle_new_tx(
        _verifier: &TxVerifier,
        _storage_adapter: &Storage,
        _tx: TxVerifier::Tx,
    ) -> Result<(), DynError> {
        Ok(())
    }
}

impl<ShareVerifier, TxVerifier, Network, DaStorage, RuntimeServiceId> ServiceData
    for GenericDaVerifierService<ShareVerifier, TxVerifier, Network, DaStorage, RuntimeServiceId>
where
    ShareVerifier: VerifierBackend,
    ShareVerifier::Settings: Clone,
    TxVerifier: TxVerifierBackend,
    TxVerifier::Settings: Clone,
    Network: NetworkAdapter<RuntimeServiceId>,
    Network::Settings: Clone,
    DaStorage: DaStorageAdapter<RuntimeServiceId>,
    DaStorage::Settings: Clone,
{
    type Settings = DaVerifierServiceSettings<
        ShareVerifier::Settings,
        TxVerifier::Settings,
        Network::Settings,
        DaStorage::Settings,
    >;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaVerifierMsg<
        <ShareVerifier::DaShare as Share>::SharesCommitments,
        <ShareVerifier::DaShare as Share>::LightShare,
        ShareVerifier::DaShare,
        (),
    >;
}

#[async_trait::async_trait]
impl<ShareVerifier, TxVerifier, Network, DaStorage, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for GenericDaVerifierService<ShareVerifier, TxVerifier, Network, DaStorage, RuntimeServiceId>
where
    ShareVerifier: VerifierBackend + Send + Sync + 'static,
    ShareVerifier::Settings: Clone + Send + Sync + 'static,
    ShareVerifier::DaShare: Debug + Send + Sync + 'static,
    ShareVerifier::Error: Error + Send + Sync + 'static,
    <ShareVerifier::DaShare as Share>::BlobId: AsRef<[u8]> + Debug + Send + Sync + 'static,
    <ShareVerifier::DaShare as Share>::LightShare: Debug + Send + Sync + 'static,
    <ShareVerifier::DaShare as Share>::SharesCommitments: Debug + Send + Sync + 'static,
    TxVerifier: TxVerifierBackend + Send + Sync + 'static,
    TxVerifier::Tx: Send,
    TxVerifier::Settings: Clone + Send + Sync + 'static,
    Network: NetworkAdapter<RuntimeServiceId, Share = ShareVerifier::DaShare, Tx = TxVerifier::Tx>
        + Send
        + Sync
        + 'static,
    Network::Membership: MembershipHandler + Clone,
    Network::Settings: Clone + Send + Sync + 'static,
    Network::Storage: MembershipStorageAdapter<
            <Network::Membership as MembershipHandler>::Id,
            <Network::Membership as MembershipHandler>::NetworkId,
        > + Send
        + Sync
        + 'static,
    Network::MembershipAdapter: MembershipAdapter,
    DaStorage:
        DaStorageAdapter<RuntimeServiceId, Share = ShareVerifier::DaShare> + Send + Sync + 'static,
    DaStorage::Settings: Clone + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Display
        + Sync
        + Send
        + 'static
        + AsServiceId<Self>
        + AsServiceId<
            NetworkService<
                Network::Backend,
                Network::Membership,
                Network::MembershipAdapter,
                Network::Storage,
                Network::ApiAdapter,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<StorageService<DaStorage::Backend, RuntimeServiceId>>,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        let DaVerifierServiceSettings {
            share_verifier_settings,
            tx_verifier_settings,
            ..
        } = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        Ok(Self {
            service_resources_handle,
            share_verifier: ShareVerifier::new(share_verifier_settings),
            tx_verifier: TxVerifier::new(tx_verifier_settings),
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
            share_verifier,
            tx_verifier,
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
            .relay::<NetworkService<_, _, _, _, _, _>>()
            .await?;
        let network_adapter = Network::new(network_adapter_settings, network_relay).await;
        let mut share_stream = network_adapter.share_stream().await;
        let mut tx_stream = network_adapter.tx_stream().await;

        let storage_relay = service_resources_handle
            .overwatch_handle
            .relay::<StorageService<_, _>>()
            .await?;
        let storage_adapter = DaStorage::new(storage_relay).await;

        service_resources_handle.status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _, _, _, _, _>,
            StorageService<_, _>
        )
        .await?;

        loop {
            tokio::select! {
                Some(share) = share_stream.next() => {
                    let blob_id = share.blob_id();
                    if let Err(err) =  Self::handle_new_share(&share_verifier, &storage_adapter, share).await {
                        error!("Error handling blob {blob_id:?} due to {err:?}");
                    }
                }
                Some(tx) = tx_stream.next() => {
                    if let Err(err) =  Self::handle_new_tx(&tx_verifier, &storage_adapter, tx).await {
                        error!("Error handling tx due to {err:?}");
                    }
                }
                Some(msg) = service_resources_handle.inbound_relay.recv() => {
                    match msg {
                        DaVerifierMsg::AddShare { share, reply_channel } => {
                            let blob_id = share.blob_id();
                            match Self::handle_new_share(&share_verifier, &storage_adapter, share).await {
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
                            match share_verifier.verify(&commitments, &light_share) {
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
pub struct DaVerifierServiceSettings<
    ShareVerifierSettings,
    TxVerifierSettings,
    NetworkSettings,
    StorageSettings,
> {
    pub share_verifier_settings: ShareVerifierSettings,
    pub tx_verifier_settings: TxVerifierSettings,
    pub network_adapter_settings: NetworkSettings,
    pub storage_adapter_settings: StorageSettings,
}
