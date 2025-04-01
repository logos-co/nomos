use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use kzgrs_backend::{
    common::build_blob_id,
    dispersal, encoder,
    encoder::{DaEncoderParams, EncodedData},
};
use nomos_core::da::{BlobId, DaDispersal, DaEncoder};
use nomos_mempool::backend::MempoolError;
use nomos_tracing::info_with_id;
use nomos_utils::bounded_duration::{MinimalBoundedDuration, NANO};
use overwatch::DynError;
use rand::{seq::IteratorRandom, thread_rng};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use tokio::time::error::Elapsed;
use tracing::instrument;

use crate::{
    adapters::{
        mempool::{DaMempoolAdapter, DaMempoolAdapterError},
        network::DispersalNetworkAdapter,
    },
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

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Timeout {
    #[serde_as(as = "MinimalBoundedDuration<1, NANO>")]
    pub wait_duration: Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MempoolPublishStrategy {
    Immediately,
    Timeout(Timeout),
    SampleSubnetworks(SampleSubnetworks),
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
    pub mempool_strategy: MempoolPublishStrategy,
}

pub struct DispersalKZGRSBackend<NetworkAdapter, MempoolAdapter> {
    settings: DispersalKZGRSBackendSettings,
    network_adapter: Arc<NetworkAdapter>,
    mempool_adapter: MempoolAdapter,
    encoder: Arc<encoder::DaEncoder>,
}

pub struct DispersalFromAdapter<Adapter> {
    adapter: Arc<Adapter>,
    timeout: Duration,
}

#[expect(
    dependency_on_unit_never_type_fallback,
    reason = "TODO: Remove if solved, this occurs in the timeout method below (out of our handling)"
)]
#[async_trait::async_trait]
impl<Adapter> DaDispersal for DispersalFromAdapter<Adapter>
where
    Adapter: DispersalNetworkAdapter + Send + Sync,
    Adapter::SubnetworkId: From<u16> + Send + Sync,
{
    type EncodedData = EncodedData;
    type Error = DynError;

    async fn disperse(&self, encoded_data: Self::EncodedData) -> Result<(), Self::Error> {
        let adapter = self.adapter.as_ref();
        let num_columns = encoded_data.column_commitments.len();
        let blob_id = build_blob_id(
            &encoded_data.aggregated_column_commitment,
            &encoded_data.row_commitments,
        );

        let responses_stream = adapter.dispersal_events_stream().await?;
        for (subnetwork_id, share) in encoded_data.into_iter().enumerate() {
            adapter
                .disperse((subnetwork_id as u16).into(), share)
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
            .collect();
        // timeout when collecting positive responses
        tokio::time::timeout(self.timeout, valid_responses)
            .await
            .map_err(|e| Box::new(e) as DynError)?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl<NetworkAdapter, MempoolAdapter> DispersalBackend
    for DispersalKZGRSBackend<NetworkAdapter, MempoolAdapter>
where
    NetworkAdapter: DispersalNetworkAdapter + Send + Sync,
    NetworkAdapter::SubnetworkId: From<u16> + Send + Sync,
    MempoolAdapter: DaMempoolAdapter<BlobId = BlobId, Metadata = dispersal::Metadata> + Send + Sync,
{
    type Settings = DispersalKZGRSBackendSettings;
    type Encoder = encoder::DaEncoder;
    type Dispersal = DispersalFromAdapter<NetworkAdapter>;
    type NetworkAdapter = NetworkAdapter;
    type MempoolAdapter = MempoolAdapter;
    type Metadata = dispersal::Metadata;
    type BlobId = BlobId;

    fn init(
        settings: Self::Settings,
        network_adapter: Self::NetworkAdapter,
        mempool_adapter: Self::MempoolAdapter,
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
            mempool_adapter,
            encoder: Arc::new(encoder),
        }
    }

    async fn encode(
        &self,
        data: Vec<u8>,
    ) -> Result<(Self::BlobId, <Self::Encoder as DaEncoder>::EncodedData), DynError> {
        let encoder = Arc::clone(&self.encoder);
        // this is a REALLY heavy task, so we should try not to block the thread here
        let heavy_task = tokio::task::spawn_blocking(move || encoder.encode(&data));
        let encoded_data = heavy_task.await??;
        let blob_id = build_blob_id(
            &encoded_data.aggregated_column_commitment,
            &encoded_data.row_commitments,
        );
        Ok((blob_id, encoded_data))
    }

    async fn disperse(
        &self,
        encoded_data: <Self::Encoder as DaEncoder>::EncodedData,
    ) -> Result<(), DynError> {
        DispersalFromAdapter {
            adapter: Arc::clone(&self.network_adapter),
            timeout: self.settings.dispersal_timeout,
        }
        .disperse(encoded_data)
        .await
    }

    async fn publish_to_mempool(
        &self,
        blob_id: Self::BlobId,
        metadata: Self::Metadata,
    ) -> Result<(), DynError> {
        self.mempool_adapter
            .post_blob_id(blob_id, metadata)
            .await
            .or_else(|err| match err {
                DaMempoolAdapterError::Mempool(MempoolError::ExistingItem) => Ok(()),
                DaMempoolAdapterError::Mempool(MempoolError::DynamicPoolError(err))
                | DaMempoolAdapterError::Other(err) => Err(err),
            })
    }

    #[instrument(skip_all)]
    async fn process_dispersal(
        &self,
        data: Vec<u8>,
        metadata: Self::Metadata,
    ) -> Result<(), DynError> {
        let (blob_id, encoded_data) = self.encode(data).await?;
        info_with_id!(blob_id.as_ref(), "ProcessDispersal");
        self.disperse(encoded_data).await?;
        match self.settings.mempool_strategy {
            MempoolPublishStrategy::Immediately => {
                self.publish_to_mempool(blob_id, metadata).await?;
            }
            // MempoolPublishStrategy::Timeout { wait_duration } => {
            MempoolPublishStrategy::Timeout(Timeout { wait_duration }) => {
                tokio::time::sleep(wait_duration).await;
                self.publish_to_mempool(blob_id, metadata).await?;
            }
            MempoolPublishStrategy::SampleSubnetworks(SampleSubnetworks {
                sample_threshold,
                timeout,
                cooldown,
            }) => {
                let subnets = {
                    // ThreadRng is not Send, need to drop before await bound.
                    let mut rng = thread_rng();
                    (0..self.settings.encoder_settings.num_columns as u16)
                        .choose_multiple(&mut rng, sample_threshold)
                };

                match tokio::time::timeout(
                    timeout,
                    self.network_adapter
                        .get_blob_samples(blob_id, &subnets, cooldown),
                )
                .await
                {
                    Ok(Ok(())) => {
                        self.publish_to_mempool(blob_id, metadata).await?;
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(Elapsed { .. }) => {
                        return Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            format!("Dispersed blob sampling timed out for blob_id {blob_id:?}"),
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}
