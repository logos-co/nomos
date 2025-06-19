use std::error::Error;

use nomos_core::da::BlobId;
use nomos_da_sampling::DaSamplingServiceMsg;
use nomos_mantle_core::{ops::Op, tx::SignedMantleTx};
use overwatch::services::{relay::OutboundRelay, ServiceData};

use super::PayloadProcessor;

#[derive(thiserror::Error, Debug)]
pub enum SignedTxProcessorError {
    #[error("Error from sampling relay {0}")]
    Sampling(Box<dyn Error>),
}

impl SignedTxProcessorError {
    fn sampling_error(err: impl Error + 'static) -> Self {
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

    async fn process(&self, payload: &Self::Payload) -> Result<(), Self::Error> {
        for op in &payload.mantle_tx.ops {
            if let Op::Blob(blob_op) = op {
                self.sampling_relay
                    .send(DaSamplingServiceMsg::TriggerSampling {
                        blob_id: blob_op.blob,
                    })
                    .await
                    .map_err(|(e, _)| SignedTxProcessorError::sampling_error(e))?;
            }
        }
        Ok(())
    }
}
