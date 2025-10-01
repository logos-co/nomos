use core::{
    convert::Infallible,
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use chain_service::{ConsensusMsg, EpochState, Slot};
use nomos_blend_message::crypto::{
    keys::Ed25519PrivateKey,
    proofs::{
        quota::{ProofOfQuota, inputs::prove::PublicInputs},
        selection::{ProofOfSelection, inputs::VerifyInputs},
    },
    random_sized_bytes,
};
use nomos_blend_scheduling::message_blend::{BlendLayerProof, SessionInfo};
use nomos_blend_service::{
    ProofsGenerator, ProofsVerifier, epoch::ChainApi, membership::service::Adapter,
};
use nomos_core::{codec::SerdeOp as _, crypto::ZkHash, mantle::SignedMantleTx};
use nomos_libp2p::PeerId;
use nomos_time::backends::NtpTimeBackend;
use overwatch::{
    overwatch::OverwatchHandle,
    services::{AsServiceId, relay::OutboundRelay},
};
use services_utils::wait_until_services_are_ready;
use tokio::sync::oneshot;

use crate::generic_services::{CryptarchiaService, MembershipService};

// TODO: Replace this with the actual verifier once the verification inputs are
// successfully fetched by the Blend service.
#[derive(Clone)]
pub struct BlendProofsVerifier;

impl ProofsVerifier for BlendProofsVerifier {
    type Error = Infallible;

    fn new() -> Self {
        Self
    }

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _inputs: &PublicInputs,
    ) -> Result<ZkHash, Self::Error> {
        use groth16::Field as _;

        Ok(ZkHash::ZERO)
    }

    fn verify_proof_of_selection(
        &self,
        _proof: ProofOfSelection,
        _inputs: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub struct BlendProofsGenerator {
    membership_size: usize,
    local_node_index: Option<usize>,
}

#[async_trait]
impl ProofsGenerator for BlendProofsGenerator {
    fn new(
        SessionInfo {
            membership_size,
            local_node_index,
            ..
        }: SessionInfo,
    ) -> Self {
        Self {
            membership_size,
            local_node_index,
        }
    }

    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof> {
        Some(loop_until_valid_proof(
            self.membership_size,
            self.local_node_index,
        ))
    }

    async fn get_next_leadership_proof(&mut self) -> Option<BlendLayerProof> {
        Some(loop_until_valid_proof(
            self.membership_size,
            self.local_node_index,
        ))
    }
}

fn loop_until_valid_proof(
    membership_size: usize,
    local_node_index: Option<usize>,
) -> BlendLayerProof {
    // For tests, we avoid generating proofs that are addressed to the local node
    // itself, since in most tests there are only two nodes and those payload would
    // fail to be propagated.
    // This is not a precaution we need to consider in production, since there will
    // be a minimum network size that is larger than 2.
    loop {
        let Ok(proof_of_quota) =
            ProofOfQuota::deserialize(&random_sized_bytes::<{ size_of::<ProofOfQuota>() }>()[..])
        else {
            continue;
        };
        let Ok(proof_of_selection) = ProofOfSelection::deserialize::<ProofOfSelection>(
            &random_sized_bytes::<{ size_of::<ProofOfSelection>() }>()[..],
        ) else {
            continue;
        };
        let Ok(expected_index) = proof_of_selection.expected_index(membership_size) else {
            continue;
        };
        if Some(expected_index) != local_node_index {
            return BlendLayerProof {
                ephemeral_signing_key: Ed25519PrivateKey::generate(),
                proof_of_quota,
                proof_of_selection,
            };
        }
    }
}

pub struct ChainService<CryptarchiaSamplingAdapter, OutboundMsg>(
    OutboundRelay<OutboundMsg>,
    PhantomData<CryptarchiaSamplingAdapter>,
);

impl<CryptarchiaSamplingAdapter, OutboundMsg> Clone
    for ChainService<CryptarchiaSamplingAdapter, OutboundMsg>
{
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1)
    }
}

impl<CryptarchiaSamplingAdapter, OutboundMsg>
    ChainService<CryptarchiaSamplingAdapter, OutboundMsg>
{
    const fn new(outbound_relay: OutboundRelay<OutboundMsg>) -> Self {
        Self(outbound_relay, PhantomData)
    }
}

#[async_trait]
impl<CryptarchiaSamplingAdapter, RuntimeServiceId> ChainApi<RuntimeServiceId>
    for ChainService<CryptarchiaSamplingAdapter, ConsensusMsg<SignedMantleTx>>
where
    CryptarchiaSamplingAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Sync,
    RuntimeServiceId: Debug
        + Display
        + Send
        + Sync
        + 'static
        + AsServiceId<CryptarchiaService<CryptarchiaSamplingAdapter, RuntimeServiceId>>,
{
    async fn new(overwatch_handle: &OverwatchHandle<RuntimeServiceId>) -> Self {
        wait_until_services_are_ready!(overwatch_handle, Some(Duration::from_secs(60)), CryptarchiaService<_, _>).await.expect("Failed to wait that the chain service is ready.");
        let outbound_relay = overwatch_handle
            .relay::<CryptarchiaService<_, _>>()
            .await
            .expect("Could not get outbound relay from Blend service to chain service.");
        Self::new(outbound_relay)
    }

    async fn get_epoch_state_for_slot(&self, slot: Slot) -> Option<EpochState> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(ConsensusMsg::GetEpochState { slot, tx: sender })
            .await
            .expect("Should not fail to send `GetEpochState` message to chain service.");
        receiver
            .await
            .expect("Should not fail to receive epoch for requested slot.")
    }
}

pub type BlendMembershipAdapter<RuntimeServiceId> =
    Adapter<MembershipService<RuntimeServiceId>, PeerId>;
pub type BlendCoreService<ChainSamplingAdapter, RuntimeServiceId> =
    nomos_blend_service::core::BlendService<
        nomos_blend_service::core::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
        BlendMembershipAdapter<RuntimeServiceId>,
        BlendProofsGenerator,
        BlendProofsVerifier,
        NtpTimeBackend,
        ChainService<ChainSamplingAdapter, ConsensusMsg<SignedMantleTx>>,
        RuntimeServiceId,
    >;
pub type BlendEdgeService<ChainSamplingAdapter, RuntimeServiceId> = nomos_blend_service::edge::BlendService<
        nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        <nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as nomos_blend_service::core::network::NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
        BlendMembershipAdapter<RuntimeServiceId>,
        BlendProofsGenerator,
        NtpTimeBackend,
        ChainService<ChainSamplingAdapter, ConsensusMsg<SignedMantleTx>>,
        RuntimeServiceId
    >;
pub type BlendService<ChainSamplingAdapter, RuntimeServiceId> = nomos_blend_service::BlendService<
    BlendCoreService<ChainSamplingAdapter, RuntimeServiceId>,
    BlendEdgeService<ChainSamplingAdapter, RuntimeServiceId>,
    RuntimeServiceId,
>;
