use core::{
    convert::Infallible,
    fmt::{Debug, Display},
    future::ready,
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use chain_leader::LeaderMsg;
use futures::{Stream, StreamExt as _};
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
    ProofOfLeadershipQuotaInputs, ProofsGenerator, ProofsVerifier,
    epoch_info::{PolEpochInfo, PolInfoProvider as PolInfoProviderTrait},
    membership::service::Adapter,
};
use nomos_core::{codec::SerdeOp as _, crypto::ZkHash};
use nomos_da_sampling::network::NetworkAdapter;
use nomos_libp2p::PeerId;
use nomos_time::backends::NtpTimeBackend;
use overwatch::{overwatch::OverwatchHandle, services::AsServiceId};
use pol::{PolChainInputsData, PolWalletInputsData, PolWitnessInputsData};
use services_utils::wait_until_services_are_ready;
use tokio::sync::oneshot::channel;
use tokio_stream::wrappers::BroadcastStream;

use crate::generic_services::{
    CryptarchiaLeaderService, CryptarchiaService, MembershipService, WalletService,
};

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

pub type BlendMembershipAdapter<RuntimeServiceId> =
    Adapter<MembershipService<RuntimeServiceId>, PeerId>;
pub type BlendCoreService<SamplingAdapter, RuntimeServiceId> =
    nomos_blend_service::core::BlendService<
        nomos_blend_service::core::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
        BlendMembershipAdapter<RuntimeServiceId>,
        BlendProofsGenerator,
        BlendProofsVerifier,
        NtpTimeBackend,
        CryptarchiaService<SamplingAdapter, RuntimeServiceId>,
        PolInfoProvider<SamplingAdapter>,
        RuntimeServiceId,
    >;
pub type BlendEdgeService<SamplingAdapter, RuntimeServiceId> = nomos_blend_service::edge::BlendService<
        nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        <nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as nomos_blend_service::core::network::NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
        BlendMembershipAdapter<RuntimeServiceId>,
        BlendProofsGenerator,
        NtpTimeBackend,
        CryptarchiaService<SamplingAdapter, RuntimeServiceId>,
        PolInfoProvider<SamplingAdapter>,
        RuntimeServiceId
    >;
pub type BlendService<SamplingAdapter, RuntimeServiceId> = nomos_blend_service::BlendService<
    BlendCoreService<SamplingAdapter, RuntimeServiceId>,
    BlendEdgeService<SamplingAdapter, RuntimeServiceId>,
    RuntimeServiceId,
>;

/// The provider of a stream of winning `PoL` epoch slots for the Blend service,
/// without introducing a cyclic dependency from Blend service to chain service.
pub struct PolInfoProvider<SamplingAdapter>(PhantomData<SamplingAdapter>);

#[async_trait]
impl<SamplingAdapter, RuntimeServiceId> PolInfoProviderTrait<RuntimeServiceId>
    for PolInfoProvider<SamplingAdapter>
where
    SamplingAdapter: NetworkAdapter<RuntimeServiceId> + 'static,
    RuntimeServiceId: AsServiceId<
            CryptarchiaLeaderService<
                CryptarchiaService<SamplingAdapter, RuntimeServiceId>,
                WalletService<
                    CryptarchiaService<SamplingAdapter, RuntimeServiceId>,
                    RuntimeServiceId,
                >,
                SamplingAdapter,
                RuntimeServiceId,
            >,
        > + Debug
        + Display
        + Send
        + Sync
        + 'static,
{
    type Stream = Box<dyn Stream<Item = PolEpochInfo> + Send + Unpin>;

    async fn subscribe(
        overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
    ) -> Option<Self::Stream> {
        use groth16::Field as _;

        wait_until_services_are_ready!(
            overwatch_handle,
            Some(Duration::from_secs(3)),
            CryptarchiaLeaderService<
                CryptarchiaService<SamplingAdapter, RuntimeServiceId>,
                WalletService<
                    CryptarchiaService<SamplingAdapter, RuntimeServiceId>,
                    RuntimeServiceId,
                >,
                SamplingAdapter,
                RuntimeServiceId,
            >
        )
        .await
        .ok()?;
        let cryptarchia_service_relay = overwatch_handle
            .relay::<CryptarchiaLeaderService<
                CryptarchiaService<SamplingAdapter, RuntimeServiceId>,
                WalletService<
                    CryptarchiaService<SamplingAdapter, RuntimeServiceId>,
                    RuntimeServiceId,
                >,
                SamplingAdapter,
                RuntimeServiceId,
            >>()
            .await
            .ok()?;
        let (sender, receiver) = channel();
        cryptarchia_service_relay
            .send(LeaderMsg::WinningPolEpochSlotStreamSubscribe { sender })
            .await
            .ok()?;
        let pol_winning_slot_receiver = receiver.await.ok()?;
        Some(Box::new(
            BroadcastStream::new(pol_winning_slot_receiver).filter_map(|res| {
                let Ok(private) = res else {
                    return ready(None);
                };
                let PolWitnessInputsData {
                    chain:
                        PolChainInputsData {
                            epoch_nonce,
                            slot_number,
                            ..
                        },
                    wallet:
                        PolWalletInputsData {
                            note_value,
                            transaction_hash,
                            output_number,
                            aged_path,
                            aged_selector,
                            slot_secret,
                            slot_secret_path,
                            starting_slot,
                            ..
                        },
                } = PolWitnessInputsData::from(private);
                ready(Some(PolEpochInfo {
                    epoch_nonce,
                    poq_private_inputs: ProofOfLeadershipQuotaInputs {
                        aged_path,
                        aged_selector,
                        note_value,
                        output_number,
                        // TODO: Replace with actual value once `LeaderPrivate` will include the
                        // note secret key.
                        pol_secret_key: ZkHash::ZERO,
                        slot: slot_number,
                        slot_secret,
                        slot_secret_path,
                        starting_slot,
                        transaction_hash,
                    },
                }))
            }),
        ))
    }
}
