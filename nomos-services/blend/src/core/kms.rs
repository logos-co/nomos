use core::fmt::{Debug, Display};

use async_trait::async_trait;
use key_management_system::{
    KMSService,
    api::KmsServiceApi,
    backend::preload::{KeyId, PreloadKMSBackend},
};
use nomos_blend_message::crypto::proofs::quota::{self, ProofOfQuota, inputs::prove::{PrivateInputs, PublicInputs, private::ProofOfCoreQuotaInputs}};
use nomos_blend_scheduling::message_blend::ProofOfQuotaGenerator;
use nomos_core::crypto::ZkHash;
use overwatch::services::AsServiceId;
use poq::CorePathAndSelectors;

pub(super) trait PreloadKMSBackendKmsApiExt<RuntimeServiceId> {
    fn poq_generator(
        &self,
        key_id: KeyId,
        core_path_and_selectors: Box<CorePathAndSelectors>,
    ) -> PreloadKMSBackendKmsPoQGenerator<RuntimeServiceId>;
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
    ) -> PreloadKMSBackendKmsPoQGenerator<RuntimeServiceId> {
        PreloadKMSBackendKmsPoQGenerator {
            core_path_and_selectors: *core_path_and_selectors,
            kms_api: self.clone(),
            key_id,
        }
    }
}

#[derive(Clone)]
pub struct PreloadKMSBackendKmsPoQGenerator<RuntimeServiceId> {
    core_path_and_selectors: CorePathAndSelectors,
    kms_api: KmsServiceApi<PreloadKmsService<RuntimeServiceId>, RuntimeServiceId>,
    key_id: KeyId,
}

#[async_trait]
impl<RuntimeServiceId> ProofOfQuotaGenerator for PreloadKMSBackendKmsPoQGenerator<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<PreloadKmsService<RuntimeServiceId>> + Debug + Display + Sync,
{
    async fn generate_poq(
        &self,
        public_inputs: &PublicInputs,
        key_index: u64,
    ) -> Result<(ProofOfQuota, ZkHash), quota::Error> {
        self.kms_api
            .execute(self.key_id.clone(), Box::new(|core_sk| {}))))
            .await
            .unwrap();
        unimplemented!()
    }
}
