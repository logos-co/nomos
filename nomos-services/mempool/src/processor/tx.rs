use std::error::Error;

use futures::{stream::FuturesUnordered, StreamExt};
use nomos_core::{
    da::BlobId,
    mantle::{ops::Op, tx::SignedMantleTx},
};
use nomos_da_sampling::DaSamplingServiceMsg;
use overwatch::services::{relay::OutboundRelay, ServiceData};

use super::PayloadProcessor;

#[derive(thiserror::Error, Debug)]
pub enum SignedTxProcessorError {
    #[error("Error from sampling relay {0}")]
    Sampling(Box<dyn Error + Send>),
}

impl SignedTxProcessorError {
    fn sampling_error(err: impl Error + Send + 'static) -> Self {
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

    async fn process(&self, payload: &Self::Payload) -> Result<(), Vec<Self::Error>> {
        let all_futures: FuturesUnordered<_> = payload
            .mantle_tx
            .ops
            .iter()
            .filter_map(|op| {
                if let Op::Blob(blob_op) = op {
                    Some(async {
                        self.sampling_relay
                            .send(DaSamplingServiceMsg::TriggerSampling {
                                blob_id: blob_op.blob,
                            })
                            .await
                            .map_err(|(e, _)| SignedTxProcessorError::sampling_error(e))
                    })
                } else {
                    None
                }
            })
            .collect();

        // In this case errors are about sending the `TriggerSampling` message to the DA
        // Sampling Service. These errors do not represent the result of
        // sampling itself, but might indicate that sending the message to
        // sampling service failed.
        let errors: Vec<SignedTxProcessorError> = StreamExt::collect::<Vec<_>>(all_futures)
            .await
            .into_iter()
            .filter_map(Result::err)
            .collect();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
