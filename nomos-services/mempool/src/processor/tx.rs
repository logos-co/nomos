use std::{error::Error, time::Duration};

use futures::stream::FuturesUnordered;
use nomos_core::{
    da::BlobId,
    mantle::{ops::Op, tx::SignedMantleTx},
};
use nomos_da_sampling::DaSamplingServiceMsg;
use overwatch::services::{relay::OutboundRelay, ServiceData};

use super::{PayloadProcessor, SamplingFutureResult};

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
}

#[async_trait::async_trait]
impl<SamplingService> PayloadProcessor for SignedTxProcessor<SamplingService>
where
    SamplingService: ServiceData<Message = DaSamplingServiceMsg<BlobId>> + Send + Sync,
{
    type Payload = SignedMantleTx;
    type Settings = ();
    type Error = SignedTxProcessorError;

    type DaSamplingService = SamplingService;

    fn new(
        (): Self::Settings,
        sampling_relay: OutboundRelay<<Self::DaSamplingService as ServiceData>::Message>,
    ) -> Self {
        Self { sampling_relay }
    }

    async fn process(
        &self,
        trigger_sampling_tasks: &mut FuturesUnordered<SamplingFutureResult<Self::Error>>,
        trigger_sampling_delay: Duration,
        payload: &Self::Payload,
    ) -> Result<(), Vec<Self::Error>> {
        for op in &payload.mantle_tx.ops {
            if let Op::ChannelBlob(blob_op) = op {
                let sampling_relay = self.sampling_relay.clone();
                let blob_id = blob_op.blob;

                trigger_sampling_tasks.push(Box::pin(async move {
                    tokio::time::sleep(trigger_sampling_delay).await;
                    sampling_relay
                        .send(DaSamplingServiceMsg::TriggerSampling { blob_id })
                        .await
                        .map_err(|(e, _)| SignedTxProcessorError::sampling_error(e))
                }));
            }
        }

        Ok(())
    }
}
