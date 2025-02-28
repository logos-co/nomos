pub mod backend;
pub mod network;
pub mod storage;

use std::{
    error::Error,
    fmt::{Debug, Formatter},
};

use backend::VerifierBackend;
use network::NetworkAdapter;
use nomos_core::da::blob::Blob;
use nomos_da_network_service::NetworkService;
use nomos_storage::StorageService;
use nomos_tracing::info_with_id;
use overwatch_rs::{
    services::{
        relay::{Relay, RelayMessage},
        state::{NoOperator, NoState},
        ServiceCore, ServiceData, ServiceId,
    },
    DynError, OpaqueServiceStateHandle,
};
use serde::{Deserialize, Serialize};
use services_utils::overwatch::lifecycle;
use storage::DaStorageAdapter;
use tokio::sync::oneshot::Sender;
use tokio_stream::StreamExt;
use tracing::{error, instrument};

const DA_VERIFIER_TAG: ServiceId = "DA-Verifier";
pub enum DaVerifierMsg<B, A> {
    AddBlob {
        blob: B,
        reply_channel: Sender<Option<A>>,
    },
}

impl<B: 'static, A: 'static> Debug for DaVerifierMsg<B, A> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddBlob { .. } => {
                write!(f, "DaVerifierMsg::AddBlob")
            }
        }
    }
}

impl<B: 'static, A: 'static> RelayMessage for DaVerifierMsg<B, A> {}

pub struct DaVerifierService<Backend, N, S>
where
    Backend: VerifierBackend,
    Backend::Settings: Clone,
    Backend::DaBlob: 'static,
    Backend::Error: Error,
    N: NetworkAdapter,
    N::Settings: Clone,
    S: DaStorageAdapter,
{
    network_relay: Relay<NetworkService<N::Backend>>,
    service_state: OpaqueServiceStateHandle<Self>,
    storage_relay: Relay<StorageService<S::Backend>>,
    verifier: Backend,
}

impl<Backend, N, S> DaVerifierService<Backend, N, S>
where
    Backend: VerifierBackend + Send + Sync + 'static,
    Backend::DaBlob: Debug + Send,
    Backend::Error: Error + Send + Sync,
    Backend::Settings: Clone,
    <Backend::DaBlob as Blob>::BlobId: AsRef<[u8]>,
    N: NetworkAdapter<Blob = Backend::DaBlob> + Send + 'static,
    N::Settings: Clone,
    S: DaStorageAdapter<Blob = Backend::DaBlob> + Send + Sync + 'static,
{
    #[instrument(skip_all)]
    async fn handle_new_blob(
        verifier: &Backend,
        storage_adapter: &S,
        blob: Backend::DaBlob,
    ) -> Result<(), DynError> {
        if storage_adapter
            .get_blob(blob.id(), blob.column_idx())
            .await?
            .is_some()
        {
            info_with_id!(blob.id().as_ref(), "VerifierBlobExists");
        } else {
            info_with_id!(blob.id().as_ref(), "VerifierAddBlob");
            verifier.verify(&blob)?;
            storage_adapter.add_blob(blob).await?;
        }
        Ok(())
    }
}

impl<Backend, N, S> ServiceData for DaVerifierService<Backend, N, S>
where
    Backend: VerifierBackend,
    Backend::Settings: Clone,
    Backend::Error: Error,
    N: NetworkAdapter,
    N::Settings: Clone,
    S: DaStorageAdapter,
    S::Settings: Clone,
{
    const SERVICE_ID: ServiceId = DA_VERIFIER_TAG;
    type Settings = DaVerifierServiceSettings<Backend::Settings, N::Settings, S::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State, Self::Settings>;
    type Message = DaVerifierMsg<Backend::DaBlob, ()>;
}

#[async_trait::async_trait]
impl<Backend, N, S> ServiceCore for DaVerifierService<Backend, N, S>
where
    Backend: VerifierBackend + Send + Sync + 'static,
    Backend::Settings: Clone + Send + Sync + 'static,
    Backend::DaBlob: Debug + Send + Sync + 'static,
    Backend::Error: Error + Send + Sync + 'static,
    <Backend::DaBlob as Blob>::BlobId: AsRef<[u8]> + Debug + Send + Sync + 'static,
    N: NetworkAdapter<Blob = Backend::DaBlob> + Send + Sync + 'static,
    N::Settings: Clone + Send + Sync + 'static,
    S: DaStorageAdapter<Blob = Backend::DaBlob> + Send + Sync + 'static,
    S::Settings: Clone + Send + Sync + 'static,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        let DaVerifierServiceSettings {
            verifier_settings, ..
        } = service_state.settings_reader.get_updated_settings();
        let network_relay = service_state.overwatch_handle.relay();
        let storage_relay = service_state.overwatch_handle.relay();
        Ok(Self {
            network_relay,
            storage_relay,
            service_state,
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
            network_relay,
            storage_relay,
            mut service_state,
            verifier,
        } = self;

        let DaVerifierServiceSettings {
            network_adapter_settings,
            ..
        } = service_state.settings_reader.get_updated_settings();

        let network_relay = network_relay.connect().await?;
        let network_adapter = N::new(network_adapter_settings, network_relay).await;
        let mut blob_stream = network_adapter.blob_stream().await;

        let storage_relay = storage_relay.connect().await?;
        let storage_adapter = S::new(storage_relay).await;

        let mut lifecycle_stream = service_state.lifecycle_handle.message_stream();
        #[expect(clippy::redundant_pub_crate)]
        loop {
            tokio::select! {
                Some(blob) = blob_stream.next() => {
                    let blob_id = blob.id();
                    if let Err(err) =  Self::handle_new_blob(&verifier,&storage_adapter, blob).await {
                        error!("Error handling blob {blob_id:?} due to {err:?}");
                    }
                }
                Some(msg) = service_state.inbound_relay.recv() => {
                    let DaVerifierMsg::AddBlob { blob, reply_channel } = msg;
                    let blob_id = blob.id();
                    match Self::handle_new_blob(&verifier, &storage_adapter, blob).await {
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
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self>(&msg).await {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaVerifierServiceSettings<BackendSettings, NetworkSettings, StorageSettings> {
    pub verifier_settings: BackendSettings,
    pub network_adapter_settings: NetworkSettings,
    pub storage_adapter_settings: StorageSettings,
}
