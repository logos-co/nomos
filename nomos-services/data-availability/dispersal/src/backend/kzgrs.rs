use std::{error::Error, sync::Arc, time::Duration};

use futures::StreamExt as _;
use kzgrs_backend::{
    common::build_blob_id,
    encoder,
    encoder::{DaEncoderParams, EncodedData},
};
use nomos_core::{
    da::{BlobId, DaDispersal, DaEncoder},
    mantle::{
        ops::channel::{Ed25519PublicKey, MsgId},
        AuthenticatedMantleTx as _, Op, SignedMantleTx,
    },
};
use nomos_tracing::info_with_id;
use nomos_utils::bounded_duration::{MinimalBoundedDuration, NANO};
use overwatch::DynError;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use tracing::instrument;

use crate::{
    adapters::{network::DispersalNetworkAdapter, wallet::DaWalletAdapter},
    backend::DispersalBackend,
};

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SampleSubnetworks {
    pub sample_threshold: usize,
    #[serde_as(as = "MinimalBoundedDuration<1, NANO>")]
    pub timeout: Duration,
    #[serde_as(as = "MinimalBoundedDuration<1, NANO>")]
    pub cooldown: Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncoderSettings {
    pub num_columns: usize,
    pub with_cache: bool,
    pub global_params_path: String,
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DispersalKZGRSBackendSettings {
    pub encoder_settings: EncoderSettings,
    #[serde_as(as = "MinimalBoundedDuration<1, NANO>")]
    pub dispersal_timeout: Duration,
}

pub struct DispersalKZGRSBackend<NetworkAdapter, WalletAdapter> {
    settings: DispersalKZGRSBackendSettings,
    network_adapter: Arc<NetworkAdapter>,
    wallet_adapter: Arc<WalletAdapter>,
    encoder: Arc<encoder::DaEncoder>,
    last_dispersed_tx: Option<SignedMantleTx>,
}

pub struct DispersalHandler<NetworkAdapter, WalletAdapter> {
    network_adapter: Arc<NetworkAdapter>,
    wallet_adapter: Arc<WalletAdapter>,
    timeout: Duration,
}

#[async_trait::async_trait]
impl<NetworkAdapter, WalletAdapter> DaDispersal for DispersalHandler<NetworkAdapter, WalletAdapter>
where
    NetworkAdapter: DispersalNetworkAdapter + Send + Sync,
    NetworkAdapter::SubnetworkId: From<u16> + Send + Sync,
    WalletAdapter: DaWalletAdapter + Send + Sync,
    WalletAdapter::Error: Error + Send + Sync + 'static,
{
    type EncodedData = EncodedData;
    type Error = DynError;

    async fn disperse_shares(&self, encoded_data: Self::EncodedData) -> Result<(), Self::Error> {
        let adapter = self.network_adapter.as_ref();
        let num_columns = encoded_data.combined_column_proofs.len();
        let blob_id = build_blob_id(&encoded_data.row_commitments);

        let responses_stream = adapter.dispersal_events_stream().await?;
        for (subnetwork_id, share) in encoded_data.into_iter().enumerate() {
            adapter
                .disperse_share((subnetwork_id as u16).into(), share)
                .await?;
        }

        let valid_responses = responses_stream
            .filter_map(|event| async move {
                match event {
                    Ok((_blob_id, _)) if _blob_id == blob_id => Some(()),
                    Err(e) => {
                        tracing::error!("Error dispersing in dispersal stream: {e}");
                        None
                    }
                    _ => None,
                }
            })
            .take(num_columns)
            .collect::<()>();
        // timeout when collecting positive responses
        tokio::time::timeout(self.timeout, valid_responses)
            .await
            .map_err(|e| Box::new(e) as DynError)?;
        Ok(())
    }

    async fn disperse_tx(
        &self,
        parent_msg_id: MsgId,
        blob_id: BlobId,
        num_columns: usize,
        original_size: usize,
        signer: Ed25519PublicKey,
    ) -> Result<SignedMantleTx, Self::Error> {
        let wallet_adapter = self.wallet_adapter.as_ref();
        let network_adapter = self.network_adapter.as_ref();

        let tx = wallet_adapter
            .blob_tx(parent_msg_id, blob_id, original_size, signer)
            .map_err(Box::new)?;
        let responses_stream = network_adapter.dispersal_events_stream().await?;
        for subnetwork_id in 0..num_columns {
            network_adapter
                .disperse_tx((subnetwork_id as u16).into(), tx.clone())
                .await?;
        }

        let valid_responses = responses_stream
            .filter_map(|event| async move {
                match event {
                    Ok((_blob_id, _)) if _blob_id == blob_id => Some(()),
                    _ => None,
                }
            })
            .take(num_columns)
            .collect::<()>();
        // timeout when collecting positive responses
        tokio::time::timeout(self.timeout, valid_responses)
            .await
            .map_err(|e| Box::new(e) as DynError)?;
        Ok(tx)
    }
}

impl<NetworkAdapter, WalletAdapter> DispersalKZGRSBackend<NetworkAdapter, WalletAdapter>
where
    NetworkAdapter: DispersalNetworkAdapter + Send + Sync,
    NetworkAdapter::SubnetworkId: From<u16> + Send + Sync,
    WalletAdapter: DaWalletAdapter + Send + Sync,
    WalletAdapter::Error: Error + Send + Sync + 'static,
{
    async fn encode(
        &self,
        data: Vec<u8>,
    ) -> Result<(BlobId, <encoder::DaEncoder as DaEncoder>::EncodedData), DynError> {
        let encoder = Arc::clone(&self.encoder);
        // this is a REALLY heavy task, so we should try not to block the thread here
        let heavy_task = tokio::task::spawn_blocking(move || encoder.encode(&data));
        let encoded_data = heavy_task.await??;
        let blob_id = build_blob_id(&encoded_data.row_commitments);
        Ok((blob_id, encoded_data))
    }

    async fn disperse(
        &mut self,
        encoded_data: <encoder::DaEncoder as DaEncoder>::EncodedData,
        original_size: usize,
    ) -> Result<(), DynError> {
        let blob_id = build_blob_id(&encoded_data.row_commitments);
        let num_columns = encoded_data.combined_column_proofs.len();

        let handler = DispersalHandler {
            network_adapter: Arc::clone(&self.network_adapter),
            wallet_adapter: Arc::clone(&self.wallet_adapter),
            timeout: self.settings.dispersal_timeout,
        };
        let parent_msg_id = self
            .last_dispersed_tx
            .as_ref()
            .map_or_else(MsgId::root, |tx| {
                let Some((Op::ChannelBlob(blob_op), _)) = tx.ops_with_proof().next() else {
                    panic!("Previously sent transaction should have a blob operation");
                };
                blob_op.id()
            });
        tracing::debug!("Dispersing {blob_id:?} transaction");
        let tx = handler
            .disperse_tx(
                parent_msg_id,
                blob_id,
                num_columns,
                original_size,
                Ed25519PublicKey::from_bytes(&[0u8; 32])?, // TODO: pass key from config
            )
            .await?;
        tracing::debug!("Dispersing {blob_id:?} shares");
        handler.disperse_shares(encoded_data).await?;
        tracing::debug!("Dispersal of {blob_id:?} successful");
        self.last_dispersed_tx = Some(tx);
        Ok(())
    }
}

#[async_trait::async_trait]
impl<NetworkAdapter, WalletAdapter> DispersalBackend
    for DispersalKZGRSBackend<NetworkAdapter, WalletAdapter>
where
    NetworkAdapter: DispersalNetworkAdapter + Send + Sync,
    NetworkAdapter::SubnetworkId: From<u16> + Send + Sync,
    WalletAdapter: DaWalletAdapter + Send + Sync,
    WalletAdapter::Error: Error + Send + Sync + 'static,
{
    type Settings = DispersalKZGRSBackendSettings;
    type Encoder = encoder::DaEncoder;
    type Dispersal = DispersalHandler<NetworkAdapter, WalletAdapter>;
    type NetworkAdapter = NetworkAdapter;
    type WalletAdapter = WalletAdapter;
    type BlobId = BlobId;

    fn init(
        settings: Self::Settings,
        network_adapter: Self::NetworkAdapter,
        wallet_adapter: Self::WalletAdapter,
    ) -> Self {
        let encoder_settings = &settings.encoder_settings;
        let global_params = kzgrs_backend::global::global_parameters_from_file(
            &encoder_settings.global_params_path,
        )
        .expect("Global encoder params should be available");
        let encoder = Self::Encoder::new(DaEncoderParams::new(
            encoder_settings.num_columns,
            encoder_settings.with_cache,
            global_params,
        ));
        Self {
            settings,
            network_adapter: Arc::new(network_adapter),
            wallet_adapter: Arc::new(wallet_adapter),
            encoder: Arc::new(encoder),
            last_dispersed_tx: None,
        }
    }

    #[instrument(skip_all)]
    async fn process_dispersal(&mut self, data: Vec<u8>) -> Result<Self::BlobId, DynError> {
        let original_size = data.len();
        let (blob_id, encoded_data) = self.encode(data).await?;
        info_with_id!(blob_id.as_ref(), "ProcessDispersal");
        self.disperse(encoded_data, original_size).await?;
        Ok(blob_id)
    }
}
