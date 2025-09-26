use async_trait::async_trait;
use futures::Stream;
use nomos_blend_message::crypto::proofs::quota::inputs::prove::private::ProofOfLeadershipQuotaInputs;
use nomos_core::crypto::ZkHash;
use overwatch::overwatch::OverwatchHandle;

#[derive(Clone)]
pub struct PolEpochInfo {
    pub epoch_nonce: ZkHash,
    pub poq_private_inputs: ProofOfLeadershipQuotaInputs,
}

#[async_trait]
pub trait PolInfoProvider<RuntimeServiceId> {
    type Stream: Stream<Item = PolEpochInfo>;

    async fn subscribe(
        overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    ) -> Option<Self::Stream>;
}
