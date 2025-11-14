use core::fmt::{Debug, Display};

use async_trait::async_trait;
use futures::{executor::block_on, future::ready};
use key_management_system::{
    KMSService,
    api::KmsServiceApi,
    backend::preload::{KeyId, PreloadKMSBackend},
    keys::errors::KeyError,
};
use nomos_blend_message::crypto::proofs::quota::{self, ProofOfQuota, inputs::prove::PublicInputs};
use nomos_blend_scheduling::message_blend::CoreProofOfQuotaGenerator;
use nomos_core::crypto::ZkHash;
use overwatch::services::AsServiceId;
use poq::CorePathAndSelectors;
use tokio::{sync::oneshot, task::spawn_blocking};

const LOG_TARGET: &str = "blend::service::core::kms-poq-generator";

pub(super) type PreloadKmsService<RuntimeServiceId> =
    KMSService<PreloadKMSBackend, RuntimeServiceId>;

#[async_trait]
pub trait KmsPoQAdapter<RuntimeServiceId> {
    type CorePoQGenerator;
    type KeyId;

    fn core_poq_generator(
        &self,
        key_id: Self::KeyId,
        core_path_and_selectors: Box<CorePathAndSelectors>,
    ) -> Self::CorePoQGenerator;
}

pub type PreloadKmsServiceKeyId<RuntimeServiceId> = <KmsServiceApi<
    PreloadKmsService<RuntimeServiceId>,
    RuntimeServiceId,
> as KmsPoQAdapter<RuntimeServiceId>>::KeyId;

#[async_trait]
impl<RuntimeServiceId> KmsPoQAdapter<RuntimeServiceId>
    for KmsServiceApi<PreloadKmsService<RuntimeServiceId>, RuntimeServiceId>
{
    type CorePoQGenerator = PreloadKMSBackendCorePoQGenerator<RuntimeServiceId>;
    type KeyId = KeyId;

    fn core_poq_generator(
        &self,
        key_id: Self::KeyId,
        core_path_and_selectors: Box<CorePathAndSelectors>,
    ) -> Self::CorePoQGenerator {
        tracing::debug!(target: LOG_TARGET, "Creating KMS-based PoQ generator with key ID {key_id:?} and core path and selectors {core_path_and_selectors:?}");
        PreloadKMSBackendCorePoQGenerator {
            core_path_and_selectors: *core_path_and_selectors,
            kms_api: self.clone(),
            key_id,
        }
    }
}

#[derive(Clone)]
pub struct PreloadKMSBackendCorePoQGenerator<RuntimeServiceId> {
    core_path_and_selectors: CorePathAndSelectors,
    kms_api: KmsServiceApi<PreloadKmsService<RuntimeServiceId>, RuntimeServiceId>,
    key_id: KeyId,
}

impl<RuntimeServiceId> CoreProofOfQuotaGenerator
    for PreloadKMSBackendCorePoQGenerator<RuntimeServiceId>
where
    RuntimeServiceId:
        AsServiceId<PreloadKmsService<RuntimeServiceId>> + Debug + Display + Send + Sync + 'static,
{
    fn generate_poq(
        &self,
        public_inputs: &PublicInputs,
        key_index: u64,
    ) -> impl Future<Output = Result<(ProofOfQuota, ZkHash), quota::Error>> + Send + Sync {
        tracing::debug!(target: LOG_TARGET, "Generating KMS-based PoQ with public_inputs {public_inputs:?} and key_index {key_index:?}.");

        let kms_api = self.kms_api.clone();
        let key_id = self.key_id.clone();
        let core_path_and_selectors = self.core_path_and_selectors;

        async move {
            let (res_sender, res_receiver) = oneshot::channel();

            generate_and_send_kms_poq(
                kms_api,
                key_id,
                Box::new(*public_inputs),
                key_index,
                Box::new(core_path_and_selectors),
                res_sender,
            )
            .await;

            let poq = res_receiver
                .await
                .expect("Should not fail to get PoQ generation result from KMS.")?;
            tracing::debug!(target: LOG_TARGET, "KMS-based PoQ generation succeeded.");
            Ok(poq)
        }
    }
}

async fn generate_and_send_kms_poq<RuntimeServiceId>(
    kms_api: KmsServiceApi<PreloadKmsService<RuntimeServiceId>, RuntimeServiceId>,
    key_id: KeyId,
    public_inputs: Box<PublicInputs>,
    key_index: u64,
    core_path_and_selectors: Box<CorePathAndSelectors>,
    result_sender: oneshot::Sender<Result<(ProofOfQuota, ZkHash), quota::Error>>,
) where
    RuntimeServiceId:
        AsServiceId<PreloadKmsService<RuntimeServiceId>> + Debug + Display + Send + Sync + 'static,
{
    spawn_blocking(move || {
        block_on(async move {
            kms_api
                .execute(
                    key_id,
                    Box::new(move |core_sk| {
                        let poq_generation_result = core_sk
                            .generate_core_poq(
                                &public_inputs,
                                key_index,
                                *core_path_and_selectors,
                            )
                            .map_err(|err| {
                                let KeyError::PoQGeneration(poq_err) = err else {
                                    panic!(
                                        "PoQ generation from KMS should return a PoQ generation error, if any."
                                    );
                                };
                                poq_err
                            });
                        tracing::debug!(
                            target: LOG_TARGET,
                            "KMS-based PoQ generation result: {poq_generation_result:?}."
                        );
                        if result_sender.send(poq_generation_result).is_err() {
                            tracing::error!(
                                target: LOG_TARGET,
                                "Failed to send PoQ generated inside the KMS to the calling context."
                            );
                        }
                        Box::pin(ready(Ok(())))
                    }),
                )
                .await
                .expect("Execute API should run successfully.");
        });
    })
    .await
    .expect("Blocking worker should not panic.");
}
