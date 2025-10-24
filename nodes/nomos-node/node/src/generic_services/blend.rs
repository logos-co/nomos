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
    keys::{Ed25519PrivateKey, Ed25519PublicKey},
    proofs::{
        PoQVerificationInputsMinusSigningKey,
        quota::{
            ProofOfQuota,
            inputs::prove::{private::ProofOfCoreQuotaInputs, public::LeaderInputs},
        },
        selection::{ProofOfSelection, inputs::VerifyInputs},
    },
    random_sized_bytes,
};
use nomos_blend_scheduling::message_blend::provers::{
    BlendLayerProof, ProofsGeneratorSettings, core_and_leader::CoreAndLeaderProofsGenerator,
    leader::LeaderProofsGenerator,
};
use nomos_blend_service::{
    ProofOfLeadershipQuotaInputs, ProofsVerifier,
    epoch_info::{PolEpochInfo, PolInfoProvider as PolInfoProviderTrait},
    membership::service::Adapter,
};
use nomos_core::{codec::DeserializeOp as _, crypto::ZkHash};
use nomos_da_sampling::network::NetworkAdapter;
use nomos_libp2p::PeerId;
use nomos_time::backends::NtpTimeBackend;
use overwatch::{overwatch::OverwatchHandle, services::AsServiceId};
use pol::{PolChainInputsData, PolWalletInputsData, PolWitnessInputsData};
use poq::{AGED_NOTE_MERKLE_TREE_HEIGHT, SLOT_SECRET_MERKLE_TREE_HEIGHT};
use services_utils::wait_until_services_are_ready;
use tokio::sync::oneshot::channel;
use tokio_stream::wrappers::WatchStream;

use crate::generic_services::{
    CryptarchiaLeaderService, CryptarchiaService, MembershipService, WalletService,
};

// TODO: Replace this with the actual verifier once the verification inputs are
// successfully fetched by the Blend service.
#[derive(Clone)]
pub struct BlendProofsVerifier;

impl ProofsVerifier for BlendProofsVerifier {
    type Error = Infallible;

    fn new(_public_inputs: PoQVerificationInputsMinusSigningKey) -> Self {
        Self
    }

    fn start_epoch_transition(&mut self, _new_pol_inputs: LeaderInputs) {}

    fn complete_epoch_transition(&mut self) {}

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _signing_key: &Ed25519PublicKey,
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
impl CoreAndLeaderProofsGenerator for BlendProofsGenerator {
    fn new(
        ProofsGeneratorSettings {
            local_node_index,
            membership_size,
            ..
        }: ProofsGeneratorSettings,
        _private_inputs: ProofOfCoreQuotaInputs,
    ) -> Self {
        Self {
            membership_size,
            local_node_index,
        }
    }

    fn rotate_epoch(&mut self, _new_epoch_public: LeaderInputs) {}

    fn set_epoch_private(&mut self, _new_epoch_private: ProofOfLeadershipQuotaInputs) {}

    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof> {
        Some(loop_until_valid_proof(
            self.membership_size,
            self.local_node_index,
        ))
    }

    async fn get_next_leader_proof(&mut self) -> Option<BlendLayerProof> {
        Some(loop_until_valid_proof(
            self.membership_size,
            self.local_node_index,
        ))
    }
}

#[async_trait]
impl LeaderProofsGenerator for BlendProofsGenerator {
    fn new(
        ProofsGeneratorSettings {
            local_node_index,
            membership_size,
            ..
        }: ProofsGeneratorSettings,
        _private_inputs: ProofOfLeadershipQuotaInputs,
    ) -> Self {
        Self {
            membership_size,
            local_node_index,
        }
    }

    fn rotate_epoch(
        &mut self,
        _new_epoch_public: LeaderInputs,
        _new_private_inputs: ProofOfLeadershipQuotaInputs,
    ) {
    }

    async fn get_next_proof(&mut self) -> BlendLayerProof {
        loop_until_valid_proof(self.membership_size, self.local_node_index)
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
            ProofOfQuota::from_bytes(&random_sized_bytes::<{ size_of::<ProofOfQuota>() }>()[..])
        else {
            continue;
        };
        let Ok(proof_of_selection) = ProofOfSelection::from_bytes(
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
            Some(Duration::from_secs(60)),
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
        // Return a `WatchStream` that filters out `None`s (i.e., at the very beginning
        // of chain leader start).
        Some(Box::new(
            WatchStream::new(pol_winning_slot_receiver)
                .filter_map(ready)
                .map(|(leader_private, secret_key, _)| {
                    let PolWitnessInputsData {
                        wallet:
                            PolWalletInputsData {
                                aged_path,
                                aged_selector,
                                note_value,
                                output_number,
                                slot_secret,
                                slot_secret_path,
                                starting_slot,
                                transaction_hash,
                                ..
                            },
                        chain: PolChainInputsData { slot_number, epoch_nonce, .. },
                    } = leader_private.input();

                // TODO: Remove this if `PoL` stuff also migrates to using fixed-size arrays or starts using vecs of the expected length instead of empty ones when generating `LeaderPrivate` values.
                let aged_path_and_selectors = {
                    let mut vec_from_inputs: Vec<_> = aged_path.iter().copied().zip(aged_selector.iter().copied()).collect();
                    let input_len = vec_from_inputs.len();
                    if input_len != AGED_NOTE_MERKLE_TREE_HEIGHT {
                        tracing::warn!("Provided merkle path for aged notes does not match the expected size for PoQ inputs.");
                    }
                    vec_from_inputs.resize(AGED_NOTE_MERKLE_TREE_HEIGHT, (ZkHash::ZERO, false));
                    vec_from_inputs
                };
                let mapped_slot_secret_path = {
                    let mut vec_from_inputs: Vec<_> = slot_secret_path.clone();
                    let input_len = vec_from_inputs.len();
                    if input_len != AGED_NOTE_MERKLE_TREE_HEIGHT {
                        tracing::warn!("Provided merkle path for slot secret does not match the expected size for PoQ inputs.");
                    }
                    vec_from_inputs.resize(SLOT_SECRET_MERKLE_TREE_HEIGHT, ZkHash::ZERO);
                    vec_from_inputs
                };

                PolEpochInfo {
                    nonce: *epoch_nonce,
                    poq_private_inputs: ProofOfLeadershipQuotaInputs {
                        aged_path_and_selectors: aged_path_and_selectors.try_into().expect("List of aged note paths and selectors does not match the expected size for PoQ inputs, although it has already been pre-processed."),
                        note_value: *note_value,
                        output_number: *output_number,
                        pol_secret_key: *secret_key.as_fr(),
                        slot: *slot_number,
                        slot_secret: *slot_secret,
                        slot_secret_path: mapped_slot_secret_path.try_into().expect("Slot secret path does not match the expected size for PoQ inputs, although it has already been pre-processed."),
                        starting_slot: *starting_slot,
                        transaction_hash: *transaction_hash,
                    },
                }
            }),
        ))
    }
}
