use core::{
    fmt::{Debug, Display},
    pin::Pin,
};

use futures::future::ready;
use key_management_system::{
    KMSService,
    api::KmsServiceApi,
    backend::preload::{KeyId, PreloadKMSBackend},
    keys::errors::KeyError,
};
use nomos_blend_message::crypto::proofs::quota::{self, ProofOfQuota, inputs::prove::PublicInputs};
use nomos_blend_scheduling::message_blend::ProofOfQuotaGenerator;
use nomos_core::crypto::ZkHash;
use overwatch::services::AsServiceId;
use poq::CorePathAndSelectors;
use tokio::sync::oneshot;

pub(super) trait PreloadKMSBackendKmsApiExt<RuntimeServiceId> {
    fn poq_generator(
        &self,
        key_id: KeyId,
        core_path_and_selectors: Box<CorePathAndSelectors>,
    ) -> PreloadKMSBackendPoQGenerator<RuntimeServiceId>;
}

pub(super) type PreloadKmsService<RuntimeServiceId> =
    KMSService<PreloadKMSBackend, RuntimeServiceId>;

impl<RuntimeServiceId> PreloadKMSBackendKmsApiExt<RuntimeServiceId>
    for KmsServiceApi<PreloadKmsService<RuntimeServiceId>, RuntimeServiceId>
{
    fn poq_generator(
        &self,
        key_id: KeyId,
        core_path_and_selectors: Box<CorePathAndSelectors>,
    ) -> PreloadKMSBackendPoQGenerator<RuntimeServiceId> {
        PreloadKMSBackendPoQGenerator {
            core_path_and_selectors: *core_path_and_selectors,
            kms_api: self.clone(),
            key_id,
        }
    }
}

#[derive(Clone)]
pub struct PreloadKMSBackendPoQGenerator<RuntimeServiceId> {
    core_path_and_selectors: CorePathAndSelectors,
    kms_api: KmsServiceApi<PreloadKmsService<RuntimeServiceId>, RuntimeServiceId>,
    key_id: KeyId,
}

impl<RuntimeServiceId> ProofOfQuotaGenerator for PreloadKMSBackendPoQGenerator<RuntimeServiceId>
where
    RuntimeServiceId:
        AsServiceId<PreloadKmsService<RuntimeServiceId>> + Debug + Display + Send + Sync,
{
    fn generate_poq(
        &self,
        public_inputs: &PublicInputs,
        key_index: u64,
    ) -> impl Future<Output = Result<(ProofOfQuota, ZkHash), quota::Error>> + Send + Sync {
        let public_inputs = *public_inputs;
        let core_path_and_selectors = self.core_path_and_selectors;
        let kms_api = self.kms_api.clone();
        let key_id = self.key_id.clone();

        async move {
            let (res_sender, res_receiver) = oneshot::channel();

            kms_api
            .execute(
                key_id,
                Box::new(move |core_sk| {
                    let poq_generation_result = core_sk.generate_core_poq(
                        &public_inputs,
                        key_index,
                        core_path_and_selectors,
                    );
                    if res_sender.send(poq_generation_result).is_err() {
                        tracing::error!(
                            "Failed to send PoQ generated inside the KMS to the calling context."
                        );
                    }
                    Box::pin(ready(Ok(())))
                        as Pin<Box<dyn Future<Output = Result<(), _>> + Send + Sync>>
                }),
            )
            .await
            .expect("Execute API should run successfully.");

            res_receiver
                .await
                .expect("Should not fail to get PoQ generation result from KMS.")
                .map_err(|e| {
                    let KeyError::PoQGeneration(poq_err) = e else {
                        panic!("PoQ generation from KMS should return a PoQ generation error.");
                    };
                    poq_err
                })
        }
    }
}
