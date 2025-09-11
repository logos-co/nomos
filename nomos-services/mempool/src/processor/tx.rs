use std::{error::Error, time::Duration};

use nomos_core::{
    da::BlobId,
    mantle::{ops::Op, tx::SignedMantleTx},
};
use nomos_da_sampling::DaSamplingServiceMsg;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::{Deserialize, Serialize};

use super::{PayloadProcessor, ProcessorTask};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedTxProcessorSettings {
    pub trigger_sampling_delay: Duration,
}

#[derive(thiserror::Error, Debug)]
pub enum SignedTxProcessorError {
    #[error("Error from sampling relay {0}")]
    Sampling(Box<dyn Error + Send + Sync>),
}

impl SignedTxProcessorError {
    fn sampling_error(err: impl Error + Send + Sync + 'static) -> Self {
        Self::Sampling(Box::new(err))
    }
}

pub struct SignedTxProcessor<SamplingService>
where
    SamplingService: ServiceData,
{
    sampling_relay: OutboundRelay<<SamplingService as ServiceData>::Message>,
    settings: SignedTxProcessorSettings,
}

#[async_trait::async_trait]
impl<SamplingService> PayloadProcessor for SignedTxProcessor<SamplingService>
where
    SamplingService: ServiceData<Message = DaSamplingServiceMsg<BlobId>> + Send + Sync,
{
    type Payload = SignedMantleTx;
    type Settings = SignedTxProcessorSettings;
    type Error = SignedTxProcessorError;

    type DaSamplingService = SamplingService;

    fn new(
        settings: Self::Settings,
        sampling_relay: OutboundRelay<<Self::DaSamplingService as ServiceData>::Message>,
    ) -> Self {
        Self {
            sampling_relay,
            settings,
        }
    }

    async fn process(
        &self,
        payload: &Self::Payload,
    ) -> Result<Vec<ProcessorTask<Self::Error>>, Vec<Self::Error>> {
        let tasks = payload
            .mantle_tx
            .ops
            .iter()
            .filter_map(|op| {
                if let Op::ChannelBlob(blob_op) = op {
                    Some(blob_op)
                } else {
                    None
                }
            })
            .map(|blob_op| {
                let sampling_relay = self.sampling_relay.clone();
                let trigger_sampling_delay = self.settings.trigger_sampling_delay;
                let blob_id = blob_op.blob;

                Box::pin(async move {
                    tokio::time::sleep(trigger_sampling_delay).await;
                    sampling_relay
                        .send(DaSamplingServiceMsg::TriggerSampling { blob_id })
                        .await
                        .map_err(|(e, _)| SignedTxProcessorError::sampling_error(e))
                }) as ProcessorTask<Self::Error>
            })
            .collect::<Vec<_>>();

        Ok(tasks)
    }
}
