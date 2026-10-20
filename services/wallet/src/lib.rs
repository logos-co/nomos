pub mod api;
mod states;

// TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
// use std::{collections::HashMap, num::NonZeroU64, time::Duration};
use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt as _, TryStreamExt as _};
use lb_chain_service::{
    ChainServiceInfo, Epoch, LibUpdate, Slot,
    api::{CryptarchiaServiceApi, CryptarchiaServiceData},
};
use lb_core::{
    block::Block,
    events::Events,
    header::HeaderId,
    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    // mantle::{
    //     NoteId, Op, OpProof, SignedOps, TxHash, Utxo, Value, VerificationError,
    //     ops::{
    //         leader_claim::{
    //             LeaderClaimOp, RewardsRoot, VoucherCm, VoucherNullifier, VoucherSecret,
    //         },
    //     },
    // },
    // proofs::leader_claim_proof::{Groth16LeaderClaimProof, LeaderClaimPrivate,
    // LeaderClaimPublic},
    // TODO(RFC-480): Operations are proven by ZkTransfers built from the
    // wallet's paths; `Inputs`, `ChannelConfigOp`, `InscriptionOp` and the SDP
    // Operation types are no longer named here.
    mantle::{
        Note, NoteId, Op, OpProof, SignedOps, TxHash, Value, VerificationError,
        gas::{GasCost, GasOverflow, MainnetGasProfile},
        ledger::verification_mode::StandardMode,
        note_set::{LEDGER_SET, NoteSetIndex, SDP_SET},
        notes::{NoteCm, NoteNf},
        ops::{
            ChannelInscribeProof, NoOpProof, OpRef, ZkAndEd25519Proof, ZkTransferProof,
            channel::ChannelId,
        },
        traits::{GenesisTx as _, Hashable as _, MantleTx as _, SignedMantleTx, StorageSize},
        transactions::{
            MantleTxBuilder, OpProofs, Ops, TxBuilderError, builder::ZkTransferOpening,
            states::Preverified, tx_list::ops::OpsContext,
        },
    },
};
use lb_groth16::Fr;
use lb_key_management_system_service::{
    api::{KmsServiceApi, KmsServiceData},
    backend::{KMSBackend, preload::PreloadKMSBackend},
    // TODO(RFC-480): the ZkSignature scheme is removed.
    // keys::{
    //     ED25519_PUBLIC_KEY_SIZE, Ed25519Key, Ed25519PublicKey, KeyOperators, PayloadEncoding,
    //     SignatureEncoding, ZkPublicKey, ZkPublicKeys, ZkSignature, secured_key::SecuredKey,
    // },
    // TODO(RFC-480): Operations are proven by ZkTransfers built from the
    // wallet's paths; `ZkPublicKeys` is no longer named here.
    keys::{
        ED25519_PUBLIC_KEY_SIZE, Ed25519Key, Ed25519PublicKey, Ed25519Signature, Key, KeyOperators,
        PayloadEncoding, SignatureEncoding, ZkKey, ZkPublicKey, secured_key::SecuredKey,
    },
    operators::zk::{nullifier::DeriveNullifier, voucher::UnsafeVoucherOperator},
};
use lb_ledger::LedgerState;
use lb_log_targets::wallet;
use lb_mmr::MerklePath;
use lb_services_utils::{
    overwatch::{RecoveryData, RecoveryOperator, StorageRecoverySettings},
    wait_until_services_are_ready,
};
use lb_storage_service::{api::StorageApi, recovery::StorageRecoveryBackend};
// TODO(RFC-480): Operations are proven by ZkTransfers built from the wallet's paths.
// use lb_utils::{bounded::BoundedError, tokio::task::spawn_blocking};
use lb_utils::bounded::BoundedError;
use lb_wallet::{
    KnownNote, OwnedNote, WalletBalance, WalletBlock, WalletError,
    transaction::{OpWitness, WitnessContext, op_witness_at, op_witnesses},
};
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceCore, ServiceData, state::StateUpdater},
};
use rand::{RngCore as _, rngs::OsRng};
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    sync::{oneshot, oneshot::Sender},
    task::JoinError,
};
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};
use tracing::{debug, error, info, trace, warn};

use crate::states::{RecoveryState, ServiceState, Wallet};

type KmsBackend = PreloadKMSBackend;
type KeyId = <KmsBackend as KMSBackend>::KeyId;

const LOG_TARGET: &str = wallet::SERVICE;

#[derive(Debug, thiserror::Error)]
pub enum WalletServiceError {
    #[error("Ledger state corresponding to block {0} not found")]
    LedgerStateNotFound(HeaderId),

    #[error("Wallet state corresponding to block {0} not found")]
    FailedToFetchWalletStateForBlock(HeaderId),

    #[error("Failed to apply historical block {0} to wallet")]
    FailedToApplyBlock(HeaderId),

    #[error("Block {0} not found in storage")]
    BlockNotFoundInStorage(HeaderId),

    #[error(transparent)]
    WalletError(#[from] WalletError),

    #[error("KMS API error: {0}")]
    KmsApi(DynError),

    #[error("Cryptarchia API error: {0}")]
    CryptarchiaApi(#[from] lb_chain_service::api::ApiError),

    #[error("Channel {0:?} is missing state in ledger")]
    MissingChannelState(ChannelId),

    #[error("Declaration {0:?} is missing in ledger")]
    MissingDeclaration(lb_core::sdp::DeclarationId),

    #[error("Service note {0:?} is missing in ledger")]
    MissingServiceNote(NoteId),

    #[error("Input note {0:?} is missing in ledger")]
    MissingInputNote(NoteId),

    #[error(transparent)]
    TxBuilder(#[from] TxBuilderError),

    #[error(transparent)]
    GasOverflow(#[from] GasOverflow),

    #[error("Transaction fee exceeded the configured max fee. tx_fee={tx_fee} > max_fee={max_fee}")]
    TxFeeExceedsMaxFee { max_fee: GasCost, tx_fee: GasCost },

    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    // #[error("PoC generation failed: {0:?}")]
    // PoCGenerationFailed(#[from] lb_core::proofs::leader_claim_proof::Error),
    //
    // #[error(transparent)]
    // MerklePathWitness(#[from] lb_core::proofs::MerklePathWitnessError),
    //
    // #[error("No claimable voucher found")]
    // NoClaimableVoucher,
    //
    // #[error("Voucher not found for the nullifier")]
    // VoucherNotFound(VoucherNullifier),
    //
    // #[error("Merkle path not found for voucher_cm: {0:?}")]
    // VoucherMerklePathNotFound(VoucherCm),
    #[error("The wallet holds no key for {0:?}")]
    UnknownKey(ZkPublicKey),

    #[error("blocking task failed: {0}")]
    TaskJoin(#[from] JoinError),

    #[error("Failed to fetch Channel Withdraw proof for op index {0} from the TxBuilder")]
    ChannelMultiSigProofNotFound(usize),

    #[error("Channel steps are proven by their holders")]
    UnsupportedSteps,

    #[error("The proofs given do not make the proof of the Operation")]
    OpProofParts,

    #[error(transparent)]
    BoundedError(#[from] BoundedError),

    #[error(transparent)]
    VerificationError(#[from] VerificationError),
    #[error("Failed to generated signature for weak public key {}", hex::encode(.0))]
    InvalidSigner([u8; ED25519_PUBLIC_KEY_SIZE]),
}

#[derive(Debug)]
pub enum WalletMsg {
    GetBalance {
        tip: Option<HeaderId>,
        pk: ZkPublicKey,
        resp_tx: Sender<Result<TipResponse<Option<WalletBalance>>, WalletServiceError>>,
    },
    FundTx {
        tip: Option<HeaderId>,
        tx_builder: MantleTxBuilder,
        change_pk: ZkPublicKey,
        funding_pks: Vec<ZkPublicKey>,
        /// Percentage of the final mandatory fee reserved as a priority-fee
        /// reserve; only the unused reserve becomes the effective tip.
        priority_fee_percent: u64,
        resp_tx: Sender<Result<TipResponse<MantleTxBuilder>, WalletServiceError>>,
    },
    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    // BuildLeaderClaimTx {
    //     tip: HeaderId,
    //     rewards_root: RewardsRoot,
    //     reward_amount: Value,
    //     funding_pk: ZkPublicKey,
    //     max_tx_fee: GasCost,
    //     resp_tx:
    //         Sender<Result<TipResponse<SignedOps<Preverified, StandardMode>>, WalletServiceError>>,
    // },
    SignTx {
        tip: Option<HeaderId>,
        tx_builder: MantleTxBuilder,
        resp_tx:
            Sender<Result<TipResponse<SignedOps<Preverified, StandardMode>>, WalletServiceError>>,
    },
    SignTxWithEd25519 {
        tx_hash: TxHash,
        pk: <Ed25519Key as SecuredKey>::PublicKey,
        resp_tx: Sender<Result<<Ed25519Key as SecuredKey>::Signature, WalletServiceError>>,
    },
    // The funding Transfer of the built Operations and its proof, None
    // without funding.
    ProveFunding {
        tip: Option<HeaderId>,
        tx_builder: MantleTxBuilder,
        resp_tx: Sender<Result<TipResponse<ProvenFunding>, WalletServiceError>>,
    },
    // Pushes a Transfer giving `owner` a note of exactly `value`.
    PrepareExactNote {
        tip: Option<HeaderId>,
        tx_builder: MantleTxBuilder,
        value: Value,
        owner: ZkPublicKey,
        funding_pks: Vec<ZkPublicKey>,
        resp_tx: Sender<Result<TipResponse<(MantleTxBuilder, KnownNote)>, WalletServiceError>>,
    },
    OwnedNote {
        tip: Option<HeaderId>,
        commitment: NoteCm,
        resp_tx: Sender<Result<TipResponse<Option<OwnedNote>>, WalletServiceError>>,
    },
    // TODO(RFC-480): the ZkSignature scheme is removed.
    // SignTxWithZk {
    //     tx_hash: TxHash,
    //     pks: ZkPublicKeys,
    //     resp_tx: Sender<Result<ZkSignature, WalletServiceError>>,
    // },
    GetLeaderAgedNotes {
        tip: Option<HeaderId>,
        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // resp_tx: Sender<Result<TipResponse<Vec<UtxoWithKeyId>>, WalletServiceError>>,
        resp_tx: Sender<Result<TipResponse<Vec<AgedNote>>, WalletServiceError>>,
    },
    GetLeaderAgedNotesInfo {
        tip: Option<HeaderId>,
        resp_tx: Sender<Result<TipResponse<LeaderAgedNotesInfo>, WalletServiceError>>,
    },
    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    // GenerateNewVoucherSecret {
    //     resp_tx: Sender<Result<VoucherCm, WalletServiceError>>,
    // },
    // GetClaimableVouchers {
    //     tip: Option<HeaderId>,
    //     resp_tx: Sender<Result<TipResponse<ClaimableVouchersInfo>, WalletServiceError>>,
    // },
    GenerateRewardKey {
        resp_tx: Sender<Result<ZkPublicKey, WalletServiceError>>,
    },
    ImportNote {
        note: Note,
        note_set: NoteSetIndex,
        resp_tx: Sender<Result<(), WalletServiceError>>,
    },
    GetKnownAddresses {
        resp_tx: Sender<Result<Vec<ZkPublicKey>, WalletServiceError>>,
    },
    GetTxContext {
        block_id: Option<HeaderId>,
        resp_tx: Sender<Result<OpsContext, WalletServiceError>>,
    },
}

// The built Operations and the proof of their funding Transfer.
pub type ProvenFunding = (Ops, Option<OpProof>);

#[derive(Debug)]
pub struct TipResponse<R> {
    pub tip: HeaderId,
    pub response: R,
}

// TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
// #[derive(Debug)]
// pub struct UtxoWithKeyId {
//     pub utxo: Utxo,
//     pub key_id: KeyId,
// }

// An owned note of the aged snapshot, with the key that spends it.
#[derive(Debug)]
pub struct AgedNote {
    pub note: OwnedNote,
    pub aged_path: MerklePath,
    pub key_id: KeyId,
}

// TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
/*
struct LeaderClaimTx {
    signed_tx: SignedOps<Preverified, StandardMode>,
    voucher_nullifier: VoucherNullifier,
    funded_notes: Vec<NoteId>,
}

struct LeaderClaimTxRequest {
    tip: HeaderId,
    rewards_root: RewardsRoot,
    reward_amount: Value,
    funding_pk: ZkPublicKey,
    max_tx_fee: GasCost,
}

#[derive(Debug)]
pub struct ClaimableVoucherInfo {
    pub commitment: VoucherCm,
    pub nullifier: VoucherNullifier,
}

/// The vouchers a wallet can claim right now, and what they are worth.
#[derive(Debug)]
pub struct ClaimableVouchersInfo {
    /// Vouchers with a proven path at the queried tip and no claim already in
    /// flight. Vouchers reserved by a pending claim are excluded.
    pub vouchers: Vec<ClaimableVoucherInfo>,
    /// What a single voucher pays out at the queried tip.
    ///
    /// The ledger splits the claimable reward pool evenly across every
    /// unclaimed voucher on the chain, so this is the same for each of
    /// `vouchers` and it moves as other leaders claim or as a new epoch adds
    /// to the pool. It is a snapshot, not a guarantee of what a claim
    /// submitted now will settle for.
    pub reward_amount: Value,
    /// `reward_amount` times the number of claimable `vouchers`: what this
    /// wallet could claim in total at the queried tip.
    pub total_claimable: Value,
}

impl ClaimableVouchersInfo {
    const fn new(vouchers: Vec<ClaimableVoucherInfo>, reward_amount: Value) -> Self {
        let total_claimable = reward_amount.saturating_mul(vouchers.len() as Value);
        Self {
            vouchers,
            reward_amount,
            total_claimable,
        }
    }
}
*/

/// One wallet-owned UTXO old enough to take part in the leadership lottery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaderAgedNoteInfo {
    // TODO(RFC-480): notes are private, named by their commitment.
    // pub note_id: NoteId,
    pub commitment: NoteCm,
    pub note_set: NoteSetIndex,
    pub value: Value,
    /// The wallet address holding the note.
    pub public_key: ZkPublicKey,
}

/// The wallet's UTXOs that are eligible to lead at a given tip.
///
/// A note is eligible when it is present in the epoch's aged UTXO snapshot —
/// the same stake distribution the leadership proof is built against — and its
/// public key is one the wallet holds a key for. An empty `notes` means this
/// node cannot win a slot at that tip: either it owns no notes, or none of
/// them have aged into the current epoch's snapshot yet.
///
/// The set is reported unfiltered. The leader service additionally skips the
/// UTXOs of the keys in `consensus_excluded_keys`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderAgedNotesInfo {
    pub notes: Vec<LeaderAgedNoteInfo>,
    /// Total value staked across `notes`, saturating.
    pub total_value: Value,
}

impl LeaderAgedNotesInfo {
    /// Number of eligible notes.
    #[must_use]
    pub const fn count(&self) -> usize {
        self.notes.len()
    }
}

// TODO(RFC-480): notes are private, named by their commitment.
/*
impl From<Vec<UtxoWithKeyId>> for LeaderAgedNotesInfo {
    fn from(utxos: Vec<UtxoWithKeyId>) -> Self {
        Self::from_iter(utxos.iter().map(|UtxoWithKeyId { utxo, .. }| utxo))
    }
}

impl<'a> FromIterator<&'a Utxo> for LeaderAgedNotesInfo {
    fn from_iter<I: IntoIterator<Item = &'a Utxo>>(utxos: I) -> Self {
        let mut total_value: Value = 0;
        let notes = utxos
            .into_iter()
            .map(|utxo| {
                total_value = total_value.saturating_add(utxo.note.value);
                LeaderAgedNoteInfo {
                    note_id: utxo.id(),
                    value: utxo.note.value,
                    public_key: utxo.note.pk,
                }
            })
            .collect();
        Self { notes, total_value }
    }
}
*/

impl From<Vec<AgedNote>> for LeaderAgedNotesInfo {
    fn from(notes: Vec<AgedNote>) -> Self {
        Self::from_iter(notes.iter().map(|AgedNote { note, .. }| note))
    }
}

impl<'a> FromIterator<&'a OwnedNote> for LeaderAgedNotesInfo {
    fn from_iter<I: IntoIterator<Item = &'a OwnedNote>>(owned_notes: I) -> Self {
        let mut total_value: Value = 0;
        let notes = owned_notes
            .into_iter()
            .map(|owned| {
                total_value = total_value.saturating_add(owned.note.value);
                LeaderAgedNoteInfo {
                    commitment: owned.note.commitment(),
                    note_set: owned.note_set,
                    value: owned.note.value,
                    public_key: owned.note.public_key,
                }
            })
            .collect();
        Self { notes, total_value }
    }
}

impl WalletMsg {
    /// Returns [`HeaderId`] of the tip if the message is associated
    /// with a specific tip.
    #[must_use]
    pub const fn tip(&self) -> Option<HeaderId> {
        match self {
            Self::GetBalance { tip, .. }
            | Self::FundTx { tip, .. }
            | Self::SignTx { tip, .. }
            | Self::ProveFunding { tip, .. }
            | Self::PrepareExactNote { tip, .. }
            | Self::OwnedNote { tip, .. }
            | Self::GetLeaderAgedNotes { tip, .. }
            | Self::GetLeaderAgedNotesInfo { tip, .. }
            // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
            // | Self::GetClaimableVouchers { tip, .. }
            | Self::GetTxContext { block_id: tip, .. } => *tip,
            // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
            // Self::BuildLeaderClaimTx { tip, .. } => Some(*tip),
            Self::SignTxWithEd25519 { .. }
            // TODO(RFC-480): the ZkSignature scheme, LEADER_CLAIM and the vouchers are removed.
            // | Self::SignTxWithZk { .. }
            // | Self::GenerateNewVoucherSecret { .. }
            | Self::GenerateRewardKey { .. }
            | Self::ImportNote { .. }
            | Self::GetKnownAddresses { .. } => None,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WalletServiceSettings {
    pub known_keys: HashMap<KeyId, ZkPublicKey>,
    pub voucher_master_key_id: KeyId,
    #[serde(skip)]
    pub recovery_data: RecoveryData,
    /// How much LIB progress a pending note reservation survives before being
    /// evicted. Notes funded into in-flight transactions are excluded from
    /// funding until they are observed spent in a block or this many immutable
    /// blocks have passed since the reservation.
    #[serde(default = "default_pending_note_expiry_blocks")]
    pub pending_note_expiry_blocks: u64,
}

#[must_use]
pub const fn default_pending_note_expiry_blocks() -> u64 {
    10
}

impl StorageRecoverySettings for WalletServiceSettings {
    const RECOVERY_KEY_SUFFIX: &'static [u8] = b"wallet";

    fn recovery_data(&self) -> &RecoveryData {
        &self.recovery_data
    }
}

pub struct WalletService<Kms, Cryptarchia, Tx, RuntimeServiceId> {
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    initial_state: RecoveryState,
    _marker: std::marker::PhantomData<(Kms, Cryptarchia, Tx)>,
}

impl<Kms, Cryptarchia, Tx, RuntimeServiceId> ServiceData
    for WalletService<Kms, Cryptarchia, Tx, RuntimeServiceId>
{
    type Settings = WalletServiceSettings;
    type State = RecoveryState;
    type StateOperator =
        RecoveryOperator<StorageRecoveryBackend<Self::State, Self::Settings, RuntimeServiceId>>;
    type Message = WalletMsg;
}

#[async_trait]
impl<Kms, Cryptarchia, Tx, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for WalletService<Kms, Cryptarchia, Tx, RuntimeServiceId>
where
    Kms: KmsServiceData<Backend = KmsBackend> + Send + Sync,
    Tx: SignedMantleTx<Preverified, StandardMode>
        + Send
        + Sync
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + StorageSize
        + 'static,
    Cryptarchia: CryptarchiaServiceData<Tx = Tx>,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<Cryptarchia>
        + AsServiceId<lb_storage_service::StorageService<RuntimeServiceId>>
        + AsServiceId<Kms>
        + std::fmt::Debug
        + std::fmt::Display
        + Send
        + Sync
        + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        initial_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_resources_handle,
            initial_state,
            _marker: std::marker::PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
        let Self {
            mut service_resources_handle,
            ..
        } = self;

        // Wait for services (except Chain) to become ready, with timeout
        wait_until_services_are_ready!(
            &service_resources_handle.overwatch_handle,
            Some(Duration::from_mins(1)),
            lb_storage_service::StorageService<_>,
            Kms
        )
        .await?;
        // Wait for Chain service to become ready, without timeout
        wait_until_services_are_ready!(
            &service_resources_handle.overwatch_handle,
            None,
            Cryptarchia // becomes ready after recoverying blocks
        )
        .await?;

        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let storage =
            StorageApi::<Tx>::from_overwatch_handle(&service_resources_handle.overwatch_handle)
                .await?;

        // Create the API wrapper for cleaner communication
        let cryptarchia_api = CryptarchiaServiceApi::<Cryptarchia>::from_overwatch_handle(
            &service_resources_handle.overwatch_handle,
        )
        .await;

        // Create KMS API for transaction signing
        let kms = KmsServiceApi::<Kms, RuntimeServiceId>::new(
            service_resources_handle
                .overwatch_handle
                .relay::<Kms>()
                .await?,
        );

        // Query chain service for current state using the API
        let ChainServiceInfo {
            cryptarchia_info, ..
        } = cryptarchia_api.info().await?;

        info!(
            target: LOG_TARGET,
            tip = ?cryptarchia_info.tip,
            lib = ?cryptarchia_info.lib,
            slot = ?cryptarchia_info.slot,
            "Wallet connecting to chain"
        );

        // Subscribe to block updates using the API. Wrapped so that lag on the
        // broadcast channel surfaces as an item rather than being dropped by a
        // `select!` pattern.
        let mut new_blocks = BroadcastStream::new(cryptarchia_api.subscribe_new_blocks().await?);

        // Subscribe to LIB updates for wallet state pruning
        let mut lib_updates = BroadcastStream::new(cryptarchia_api.subscribe_lib_updates().await?);

        let (epoch_config, consensus_config) = cryptarchia_api.get_epoch_config().await?;
        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // let security_param = NonZeroU64::from(consensus_config.security_param()).get();
        let epoch_config = EpochConfig {
            epoch_config,
            consensus_config,
        };

        // TODO(RFC-480): a wallet follows the commitment MMRs from genesis.
        // // Initialize wallet from LIB and LIB LedgerState
        // let lib = cryptarchia_info.lib;
        //
        // // Fetch the ledger state at LIB using the API
        // let lib_ledger = cryptarchia_api
        //     .get_ledger_state(lib)
        //     .await?
        //     .ok_or(WalletServiceError::LedgerStateNotFound(lib))?;
        //
        // let mut state = ServiceState::new(
        //     self.initial_state,
        //     &settings,
        //     lib,
        //     &lib_ledger,
        //     &service_resources_handle.state_updater,
        //     security_param,
        // );
        let mut state = Self::start_state(
            self.initial_state,
            &settings,
            &service_resources_handle.state_updater,
            &cryptarchia_api,
            &kms,
        )
        .await?;
        let voucher_master_key_id = settings.voucher_master_key_id;

        Self::backfill_missing_blocks(
            cryptarchia_info.tip,
            &mut state,
            &storage,
            &cryptarchia_api,
            &kms,
            &epoch_config,
        )
        .await?;

        service_resources_handle.status_updater.notify_ready();
        info!(target: LOG_TARGET, "Wallet service is ready and subscribed to blocks");

        loop {
            tokio::select! {
                Some(msg) = service_resources_handle.inbound_relay.recv() => {
                    Box::pin(Self::handle_wallet_message(msg, &mut state, &voucher_master_key_id, &storage, &cryptarchia_api, &kms, &epoch_config)).await;
                }
                // A skipped block shows up as an unknown parent on the next
                // one and is backfilled from there; the log line is what tells
                // an operator why the wallet fell behind.
                Some(event) = new_blocks.next() => match event {
                    Ok(event) => Self::handle_new_block(event.block_id, &mut state, &storage, &cryptarchia_api, &kms, &epoch_config).await,
                    Err(BroadcastStreamRecvError::Lagged(skipped)) => warn_lagged("new-block", skipped),
                },
                Some(lib_update) = lib_updates.next() => match lib_update {
                    Ok(lib_update) => Self::handle_lib_update(&lib_update, &storage, &mut state, &cryptarchia_api, &kms, &epoch_config).await,
                    Err(BroadcastStreamRecvError::Lagged(skipped)) => warn_lagged("LIB", skipped),
                },
            }
        }
    }
}

/// The wallet fell `skipped` items behind one of its chain subscriptions. Only
/// the latest LIB matters, and a skipped block is backfilled from the next one,
/// so this is a diagnostic rather than an error.
fn warn_lagged(stream: &str, skipped: u64) {
    warn!(target: LOG_TARGET, stream, skipped, "Wallet fell behind a chain subscription");
}

impl<Kms, Cryptarchia, Tx, RuntimeServiceId> WalletService<Kms, Cryptarchia, Tx, RuntimeServiceId>
where
    Kms: KmsServiceData<Backend = KmsBackend>,
    Tx: SignedMantleTx<Preverified, StandardMode>
        + Send
        + Sync
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + StorageSize
        + 'static,
    Cryptarchia: CryptarchiaServiceData<Tx = Tx> + Send + 'static,
    RuntimeServiceId:
        AsServiceId<Cryptarchia> + AsServiceId<Kms> + std::fmt::Debug + std::fmt::Display + Sync,
{
    async fn msg_tip_or_latest(
        msg_tip: Option<HeaderId>,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
    ) -> Result<HeaderId, WalletServiceError> {
        if let Some(tip) = msg_tip {
            Ok(tip)
        } else {
            let ChainServiceInfo {
                cryptarchia_info, ..
            } = cryptarchia.info().await?;
            Ok(cryptarchia_info.tip)
        }
    }

    async fn ledger_state_at(
        tip: HeaderId,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
    ) -> Result<LedgerState, WalletServiceError> {
        cryptarchia
            .get_ledger_state(tip)
            .await?
            .ok_or(WalletServiceError::LedgerStateNotFound(tip))
    }

    #[expect(clippy::too_many_lines, reason = "TODO: Address this at some point.")]
    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: address this in a dedicated refactor"
    )]
    async fn handle_wallet_message(
        msg: WalletMsg,
        state: &mut ServiceState<'_>,
        voucher_master_key_id: &KeyId,
        storage: &StorageApi<Tx>,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        epoch_config: &EpochConfig,
    ) {
        if let Err(err) =
            Self::backfill_if_not_in_sync(msg.tip(), state, storage, cryptarchia, kms, epoch_config)
                .await
        {
            warn!(
                target: LOG_TARGET,
                err = ?err,
                "Failed backfilling wallet to message tip; continuing to process the message {msg:?}"
            );
        }

        match msg {
            WalletMsg::GetBalance { tip, pk, resp_tx } => {
                Self::handle_get_balance(tip, pk, resp_tx, state.wallet(), cryptarchia).await;
            }
            WalletMsg::FundTx {
                tip,
                tx_builder,
                change_pk,
                funding_pks,
                priority_fee_percent,
                resp_tx,
            } => {
                let tip = match Self::msg_tip_or_latest(tip, cryptarchia).await {
                    Ok(tip) => tip,
                    Err(err) => {
                        Self::send_err(resp_tx, err);
                        return;
                    }
                };

                // Fetch the tx context (gas prices) fresh at fund time. It is
                // tip-dependent, so a builder handed to us earlier must be funded
                // against the current context rather than a stale one.
                let context = match Self::ledger_state_at(tip, cryptarchia).await {
                    Ok(ledger) => ledger.tx_context(),
                    Err(err) => {
                        Self::send_err(resp_tx, err);
                        return;
                    }
                };

                let change_nonce = random_nonce();
                let funded = match state.fund_tx::<MainnetGasProfile>(
                    tip,
                    &tx_builder,
                    change_pk,
                    change_nonce,
                    funding_pks,
                    &context,
                    priority_fee_percent,
                ) {
                    Ok(funded) => funded,
                    Err(err) => {
                        Self::send_err(resp_tx, WalletServiceError::from(err));
                        return;
                    }
                };

                if let Err(err) =
                    Self::add_change_note(&funded, change_pk, change_nonce, state, kms).await
                {
                    Self::send_err(resp_tx, err);
                    return;
                }

                // TODO(RFC-480): notes are reserved by nullifier.
                // let funded_notes: Vec<NoteId> =
                //     funded.notes_consumed_or_used_in_service().collect();
                let funded_notes: Vec<NoteNf> = funded.nullifiers_consumed().collect();
                state.reserve_pending_notes(funded_notes.iter().copied());

                if resp_tx
                    .send(Ok(TipResponse {
                        tip,
                        response: funded,
                    }))
                    .is_err()
                {
                    state.release_pending_notes(funded_notes);
                    debug!(target: LOG_TARGET, "Failed to respond to FundTx");
                }
            }
            // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
            /*
            WalletMsg::BuildLeaderClaimTx {
                tip,
                rewards_root,
                reward_amount,
                funding_pk,
                max_tx_fee,
                resp_tx,
            } => {
                let ledger = match Self::ledger_state_at(tip, cryptarchia).await {
                    Ok(ledger) => ledger,
                    Err(err) => {
                        Self::send_err(resp_tx, err);
                        return;
                    }
                };
                let request = LeaderClaimTxRequest {
                    tip,
                    rewards_root,
                    reward_amount,
                    funding_pk,
                    max_tx_fee,
                };
                // Pinned to keep the future off the stack: `LedgerState` is
                // passed by value and grew past the `clippy::large_futures`
                // threshold, by 720 bytes for `PoW` and 112 for the uncle
                // slots.
                // TODO: consider passing it by reference so we can remove `Box::pin`.
                let response =
                    Box::pin(Self::build_leader_claim_tx(request, ledger, state, kms)).await;

                match response {
                    Ok(built_tx) => {
                        let voucher_nullifier = built_tx.voucher_nullifier;
                        let funded_notes = built_tx.funded_notes;
                        if resp_tx
                            .send(Ok(TipResponse {
                                tip,
                                response: built_tx.signed_tx,
                            }))
                            .is_err()
                        {
                            state.release_claim_reservation(voucher_nullifier);
                            state.release_pending_notes(funded_notes);
                            debug!(target: LOG_TARGET, "Failed to respond to BuildLeaderClaimTx");
                        }
                    }
                    Err(err) => {
                        if resp_tx.send(Err(err)).is_err() {
                            debug!(target: LOG_TARGET, "Failed to respond to BuildLeaderClaimTx");
                        }
                    }
                }
            }
            */
            WalletMsg::SignTx {
                tip,
                tx_builder,
                resp_tx,
            } => {
                let tip = match Self::msg_tip_or_latest(tip, cryptarchia).await {
                    Ok(tip) => tip,
                    Err(err) => {
                        Self::send_err(resp_tx, err);
                        return;
                    }
                };

                let ledger = match Self::ledger_state_at(tip, cryptarchia).await {
                    Ok(ledger) => ledger,
                    Err(err) => {
                        Self::send_err(resp_tx, err);
                        return;
                    }
                };

                // TODO(RFC-480): notes are reserved by nullifier.
                // let funded_notes: Vec<NoteId> =
                //     tx_builder.notes_consumed_or_used_in_service().collect();
                let funded_notes: Vec<NoteNf> = tx_builder.nullifiers_consumed().collect();

                // TODO(RFC-480): the ledger state is lent to keep the proving
                // future small.
                // let resp = Self::sign_tx(tx_builder, tip, ledger, kms, state.wallet())
                let resp = Self::sign_tx(tx_builder, tip, &ledger, kms, state.wallet())
                    .await
                    .map(|signed_tx| TipResponse {
                        tip,
                        response: signed_tx,
                    });

                let signing_failed = resp.is_err();
                let response_delivered = resp_tx.send(resp).is_ok();

                if signing_failed || !response_delivered {
                    state.release_pending_notes(funded_notes);
                }

                if !response_delivered {
                    debug!(target: LOG_TARGET, "Failed to respond to SignTx");
                }
            }
            WalletMsg::ProveFunding {
                tip,
                tx_builder,
                resp_tx,
            } => {
                let tip = match Self::msg_tip_or_latest(tip, cryptarchia).await {
                    Ok(tip) => tip,
                    Err(err) => {
                        Self::send_err(resp_tx, err);
                        return;
                    }
                };

                let ledger = match Self::ledger_state_at(tip, cryptarchia).await {
                    Ok(ledger) => ledger,
                    Err(err) => {
                        Self::send_err(resp_tx, err);
                        return;
                    }
                };

                let funded_notes: Vec<NoteNf> = tx_builder.nullifiers_consumed().collect();

                let resp = Self::prove_funding(tx_builder, tip, &ledger, kms, state.wallet())
                    .await
                    .map(|response| TipResponse { tip, response });

                let proving_failed = resp.is_err();
                let response_delivered = resp_tx.send(resp).is_ok();

                if proving_failed || !response_delivered {
                    state.release_pending_notes(funded_notes);
                }

                if !response_delivered {
                    debug!(target: LOG_TARGET, "Failed to respond to ProveFunding");
                }
            }
            WalletMsg::PrepareExactNote {
                tip,
                tx_builder,
                value,
                owner,
                funding_pks,
                resp_tx,
            } => {
                let tip = match Self::msg_tip_or_latest(tip, cryptarchia).await {
                    Ok(tip) => tip,
                    Err(err) => {
                        Self::send_err(resp_tx, err);
                        return;
                    }
                };

                let (tx_builder, exact_note, split_notes) = match Self::prepare_exact_note(
                    tip,
                    tx_builder,
                    value,
                    owner,
                    funding_pks,
                    state,
                    kms,
                )
                .await
                {
                    Ok(prepared) => prepared,
                    Err(err) => {
                        Self::send_err(resp_tx, err);
                        return;
                    }
                };
                state.reserve_pending_notes(split_notes.iter().copied());

                if resp_tx
                    .send(Ok(TipResponse {
                        tip,
                        response: (tx_builder, exact_note),
                    }))
                    .is_err()
                {
                    state.release_pending_notes(split_notes);
                    debug!(target: LOG_TARGET, "Failed to respond to PrepareExactNote");
                }
            }
            WalletMsg::OwnedNote {
                tip,
                commitment,
                resp_tx,
            } => {
                let response =
                    Self::owned_note_at(tip, &commitment, state.wallet(), cryptarchia).await;
                if resp_tx.send(response).is_err() {
                    debug!(target: LOG_TARGET, "Failed to respond to OwnedNote");
                }
            }
            WalletMsg::SignTxWithEd25519 {
                tx_hash,
                pk,
                resp_tx,
            } => {
                let result = Self::sign_ed25519(tx_hash, pk, kms).await;
                if resp_tx.send(result).is_err() {
                    debug!(target: LOG_TARGET, "Failed to respond to SignTxWithEd25519");
                }
            }
            // TODO(RFC-480): the ZkSignature scheme is removed.
            // WalletMsg::SignTxWithZk {
            //     tx_hash,
            //     pks,
            //     resp_tx,
            // } => {
            //     let result = Self::sign_zksig(tx_hash, pks, kms).await;
            //     if resp_tx.send(result).is_err() {
            //         debug!(target: LOG_TARGET, "Failed to respond to SignTxWithZk");
            //     }
            // }
            WalletMsg::GetLeaderAgedNotes { tip, resp_tx } => {
                let response = Self::leader_aged_notes_at(tip, state.wallet(), cryptarchia).await;
                if resp_tx.send(response).is_err() {
                    debug!(target: LOG_TARGET, "Failed to respond to GetLeaderAgedNotes");
                }
            }
            WalletMsg::GetLeaderAgedNotesInfo { tip, resp_tx } => {
                let response = Self::leader_aged_notes_at(tip, state.wallet(), cryptarchia)
                    .await
                    .map(|TipResponse { tip, response }| TipResponse {
                        tip,
                        response: LeaderAgedNotesInfo::from(response),
                    });
                if resp_tx.send(response).is_err() {
                    debug!(target: LOG_TARGET, "Failed to respond to GetLeaderAgedNotesInfo");
                }
            }
            // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
            // WalletMsg::GenerateNewVoucherSecret { resp_tx } => {
            //     Self::generate_new_voucher_secret(
            //         state,
            //         voucher_master_key_id.clone(),
            //         kms,
            //         resp_tx,
            //     )
            //     .await;
            // }
            // WalletMsg::GetClaimableVouchers { tip, resp_tx } => {
            //     Self::get_claimable_vouchers(tip, resp_tx, state, cryptarchia).await;
            // }
            WalletMsg::GenerateRewardKey { resp_tx } => {
                let index = state.next_reward_key_index();
                let result =
                    Self::register_reward_key(state, voucher_master_key_id.clone(), index, kms)
                        .await;
                if resp_tx.send(result).is_err() {
                    debug!(target: LOG_TARGET, "Failed to respond to GenerateRewardKey");
                }
            }
            WalletMsg::ImportNote {
                note,
                note_set,
                resp_tx,
            } => {
                let result = Self::import_note(note, note_set, state, kms).await;
                if resp_tx.send(result).is_err() {
                    debug!(target: LOG_TARGET, "Failed to respond to ImportNote");
                }
            }
            WalletMsg::GetKnownAddresses { resp_tx } => {
                Self::get_known_addresses(state.wallet(), resp_tx);
            }
            WalletMsg::GetTxContext { block_id, resp_tx } => {
                Self::get_tx_context(block_id, resp_tx, cryptarchia).await;
            }
        }
    }

    async fn handle_get_balance(
        tip: Option<HeaderId>,
        pk: ZkPublicKey,
        resp_tx: Sender<Result<TipResponse<Option<WalletBalance>>, WalletServiceError>>,
        wallet: &Wallet,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
    ) {
        let tip = match Self::msg_tip_or_latest(tip, cryptarchia).await {
            Ok(tip) => tip,
            Err(err) => {
                Self::send_err(resp_tx, err);
                return;
            }
        };

        let resp = wallet
            .balance(tip, pk)
            .map_err(WalletServiceError::WalletError)
            .map(|balance| TipResponse {
                tip,
                response: balance,
            });

        if resp_tx.send(resp).is_err() {
            debug!(target: LOG_TARGET, "Failed to respond to GetBalance");
        }
    }

    // TODO(RFC-480): Operations are proven by ZkTransfers built from the wallet's
    // paths.
    /*
    async fn sign_inscription(
        tx_hash: TxHash,
        inscribe_op: &InscriptionOp,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<OpProof, WalletServiceError> {
        let Ok(validated_public_key) = Ed25519PublicKey::try_from(inscribe_op.signer) else {
            return Err(WalletServiceError::InvalidSigner(
                inscribe_op.signer.to_bytes(),
            ));
        };
        let ed25519_sig = Self::sign_ed25519(tx_hash, validated_public_key, kms).await?;
        Ok(OpProof::Ed25519Sig(ed25519_sig))
    }

    async fn sign_channel_deposit(
        tx_hash: TxHash,
        note_ids: Inputs,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        ledger: &LedgerState,
    ) -> Result<OpProof, WalletServiceError> {
        let input_pks = Self::resolve_note_input_pks(ledger, note_ids.into_inner())?;
        let zk_sig = Self::sign_zksig(tx_hash, input_pks, kms).await?;

        Ok(OpProof::ZkSig(zk_sig))
    }

    async fn sign_channel_set_key(
        tx_hash: TxHash,
        set_keys_op: &ChannelConfigOp,
        ledger: &LedgerState,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<OpProof, WalletServiceError> {
        let channel = ledger
            .mantle_ledger()
            .channels()
            .channel_state(&set_keys_op.channel)
            .ok_or(WalletServiceError::MissingChannelState(set_keys_op.channel))?;

        let authorized_key = channel.accredited_keys[0]; // First key is authorized key (guaranteed non-empty)
        let Ok(validated_public_key) = Ed25519PublicKey::try_from(authorized_key) else {
            return Err(WalletServiceError::InvalidSigner(authorized_key.to_bytes()));
        };
        let ed25519_sig = Self::sign_ed25519(tx_hash, validated_public_key, kms).await?;

        Ok(OpProof::Ed25519Sig(ed25519_sig))
    }

    async fn sign_sdp_declare(
        tx_hash: TxHash,
        declare_op: &SDPDeclareOp,
        ledger: &LedgerState,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<OpProof, WalletServiceError> {
        // For a new declaration, the note is still in the UTXOs (not yet locked).
        // We look it up from the UTXO set to get the public key for signing.
        let utxo_tree = ledger.latest_utxos();
        debug!(
            target: LOG_TARGET,
            "SDPDeclare: Looking for note_id={}, utxo_tree has {} UTXOs",
            hex::encode(declare_op.service_note_id.as_bytes()),
            utxo_tree.size()
        );
        let note = utxo_tree
            .utxos()
            .get(&declare_op.service_note_id)
            .map(|(utxo, _)| utxo.note)
            .ok_or(WalletServiceError::MissingServiceNote(
                declare_op.service_note_id,
            ))?;

        let zk_sig = Self::sign_zksig(tx_hash, [note.pk, declare_op.zk_id], kms).await?;
        let ed25519_sig = Self::sign_ed25519(tx_hash, declare_op.provider_id.0, kms).await?;

        let proof = ZkAndEd25519Proof {
            zk_sig,
            ed25519_sig,
        };
        Ok(OpProof::ZkAndEd25519Sigs(proof))
    }

    async fn sign_sdp_withdraw(
        tx_hash: TxHash,
        withdraw_op: &SDPWithdrawOp,
        ledger: &LedgerState,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<OpProof, WalletServiceError> {
        let declaration = ledger
            .mantle_ledger()
            .sdp_ledger()
            .get_declaration(&withdraw_op.declaration_id)
            .ok_or(WalletServiceError::MissingDeclaration(
                withdraw_op.declaration_id,
            ))?;

        let service_note = ledger
            .mantle_ledger()
            .service_notes()
            .get(&declaration.service_note_id)
            .ok_or(WalletServiceError::MissingServiceNote(
                declaration.service_note_id,
            ))?;

        let zk_sig = Self::sign_zksig(tx_hash, [service_note.pk, declaration.zk_id], kms).await?;

        Ok(OpProof::ZkSig(zk_sig))
    }

    async fn sign_sdp_active(
        tx_hash: TxHash,
        active_op: &SDPActiveOp,
        ledger: &LedgerState,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<OpProof, WalletServiceError> {
        let declaration = ledger
            .mantle_ledger()
            .sdp_ledger()
            .get_declaration(&active_op.declaration_id)
            .ok_or(WalletServiceError::MissingDeclaration(
                active_op.declaration_id,
            ))?;

        let zk_sig = Self::sign_zksig(tx_hash, [declaration.zk_id], kms).await?;

        Ok(OpProof::ZkSig(zk_sig))
    }

    async fn sign_leader_claim(
        tx_hash: TxHash,
        leader_claim_op: &LeaderClaimOp,
        tip: HeaderId,
        wallet: &Wallet,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<OpProof, WalletServiceError> {
        let (voucher_master_key_id, voucher_index) = wallet
            .get_voucher_by_nullifier(&leader_claim_op.voucher_nullifier)
            .ok_or(WalletServiceError::VoucherNotFound(
                leader_claim_op.voucher_nullifier,
            ))?;
        let voucher_secret =
            Self::derive_voucher_from_kms(kms, voucher_master_key_id.clone(), *voucher_index)
                .await?;

        let voucher_cm = VoucherCm::from_secret(voucher_secret);
        let path = wallet
            .voucher_path_snapshot(tip, &voucher_cm)
            .map_err(WalletServiceError::WalletError)?
            .ok_or(WalletServiceError::VoucherMerklePathNotFound(voucher_cm))?;
        let rewards_root = leader_claim_op.rewards_root;

        // TODO: This should happen in KMS
        let poc = spawn_blocking("logos/wallet/leader-claim-proof-blocking", move || {
            Self::generate_poc(voucher_secret, &path, rewards_root, tx_hash)
        })
        .await??;

        Ok(OpProof::PoC(poc))
    }

    async fn sign_transfer(
        tx_hash: TxHash,
        note_ids: Inputs,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        ledger: &LedgerState,
    ) -> Result<OpProof, WalletServiceError> {
        let input_pks = Self::resolve_note_input_pks(ledger, note_ids.into_inner())?;
        let zk_sig = Self::sign_zksig(tx_hash, input_pks, kms).await?;

        Ok(OpProof::ZkSig(zk_sig))
    }

    async fn sign_tx(
        tx_builder: MantleTxBuilder,
        tip: HeaderId,
        tip_leader: LedgerState,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        wallet: &Wallet,
    ) -> Result<SignedOps<Preverified, StandardMode>, WalletServiceError> {
        // TODO: Maybe Unverified?
        // Extract input public keys before building the transaction
        let mut channel_multi_sig_proofs = tx_builder.channel_multi_sig_proofs().clone();
        let mantle_tx = tx_builder.clone().build()?;
        let tx_hash = mantle_tx.hash();

        let mut op_proofs = OpProofs::empty();
        for (i, op) in mantle_tx.op_refs().into_iter().enumerate() {
            let op_proof = match op {
                OpRef::ChannelInscribe(inscribe_op) => {
                    Self::sign_inscription(tx_hash, inscribe_op, kms).await?
                }
                OpRef::ChannelConfig(set_keys_op) => {
                    Self::sign_channel_set_key(tx_hash, set_keys_op, &tip_leader, kms).await?
                }
                OpRef::ChannelDeposit(deposit_op) => {
                    Self::sign_channel_deposit(tx_hash, deposit_op.inputs.clone(), kms, &tip_leader)
                        .await?
                }
                OpRef::ChannelWithdraw(_) | OpRef::ChannelTransfer(_) => {
                    let proof = channel_multi_sig_proofs
                        .remove(&i)
                        .ok_or(WalletServiceError::ChannelMultiSigProofNotFound(i))?;
                    OpProof::ChannelMultiSigProof(proof)
                }
                OpRef::SDPDeclare(declare_op) => {
                    Self::sign_sdp_declare(tx_hash, declare_op, &tip_leader, kms).await?
                }
                OpRef::SDPWithdraw(withdraw_op) => {
                    Self::sign_sdp_withdraw(tx_hash, withdraw_op, &tip_leader, kms).await?
                }
                OpRef::SDPActive(active_op) => {
                    Self::sign_sdp_active(tx_hash, active_op, &tip_leader, kms).await?
                }
                OpRef::LeaderClaim(claim_op) => {
                    Self::sign_leader_claim(tx_hash, claim_op, tip, wallet, kms).await?
                }
                OpRef::Transfer(transfer_op) => {
                    Self::sign_transfer(tx_hash, transfer_op.inputs.clone(), kms, &tip_leader)
                        .await?
                }
                OpRef::ClaimPowReward(_) => OpProof::None(NoOpProof),
            };
            op_proofs.try_push(op_proof)?;
        }

        let signed_mantle_tx = SignedOps::from_parts(mantle_tx, op_proofs)
            .expect("SignedOps should be valid with the provided proofs.")
            .preverify()
            .expect("Preverification should not fail.");

        Ok(signed_mantle_tx)
    }
    */

    async fn sign_tx(
        tx_builder: MantleTxBuilder,
        tip: HeaderId,
        tip_ledger: &LedgerState,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        wallet: &Wallet,
    ) -> Result<SignedOps<Preverified, StandardMode>, WalletServiceError> {
        let mut channel_multi_sig_proofs = tx_builder.channel_multi_sig_proofs().clone();
        let openings = tx_builder.openings();
        let ops = tx_builder.build()?;
        let tx_hash = ops.hash();

        let mut zk_proofs = HashMap::new();
        for witness in Self::witnesses_at(&ops, &openings, tip, tip_ledger, wallet)? {
            zk_proofs.insert(witness.op_index, Self::prove_witness(witness, kms).await?);
        }

        let mut op_proofs = OpProofs::empty();
        for (i, op) in ops.op_refs().into_iter().enumerate() {
            let op_proof = if let OpRef::ChannelConfig(_) = op {
                let proof = channel_multi_sig_proofs
                    .remove(&i)
                    .ok_or(WalletServiceError::ChannelMultiSigProofNotFound(i))?;
                OpProof::ChannelMultiSigProof(proof)
            } else {
                let signature = Self::op_signature(tx_hash, &op, tip_ledger, kms).await?;
                op_proof(&op, zk_proofs.remove(&i), signature)?
            };
            op_proofs.try_push(op_proof)?;
        }

        let signed_mantle_tx = SignedOps::from_parts(ops, op_proofs)
            .expect("SignedOps should be valid with the provided proofs.")
            .preverify()?;

        Ok(signed_mantle_tx)
    }

    // The built Operations and the proof of their funding Transfer, the last
    // Operation, when the builder funds them.
    async fn prove_funding(
        tx_builder: MantleTxBuilder,
        tip: HeaderId,
        tip_ledger: &LedgerState,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        wallet: &Wallet,
    ) -> Result<(Ops, Option<OpProof>), WalletServiceError> {
        let funds = !tx_builder.funding_inputs().is_empty();
        let openings = tx_builder.openings();
        let ops = tx_builder.build()?;
        let Some(funding_index) = ops.len().checked_sub(1).filter(|_| funds) else {
            return Ok((ops, None));
        };

        // The other Operations are left for the caller to prove
        let witness = op_witness_at(
            &wallet.wallet_state_at(tip)?,
            wallet.known_notes(),
            &ops,
            &openings,
            &Self::witness_context(&ops, tip_ledger),
            ops.hash().to_fr(),
            funding_index,
        )?;
        let proof = Self::prove_witness(witness, kms).await?;
        Ok((ops, Some(OpProof::ZkTransfer(proof))))
    }

    // The ZkTransfer witnesses of `ops`, against the wallet's paths at `tip`.
    fn witnesses_at(
        ops: &Ops,
        openings: &BTreeMap<usize, ZkTransferOpening>,
        tip: HeaderId,
        tip_ledger: &LedgerState,
        wallet: &Wallet,
    ) -> Result<Vec<OpWitness>, WalletServiceError> {
        Ok(op_witnesses(
            &wallet.wallet_state_at(tip)?,
            wallet.known_notes(),
            ops,
            openings,
            &Self::witness_context(ops, tip_ledger),
            ops.hash().to_fr(),
        )?)
    }

    fn witness_context(ops: &Ops, tip_ledger: &LedgerState) -> WitnessContext {
        let channels = tip_ledger.mantle_ledger().channels();
        let channel_note_sets = ops
            .iter()
            .filter_map(|op| match op {
                Op::ChannelDeposit(deposit) => Some(deposit.channel_id),
                Op::ChannelWithdraw(withdraw) => Some(withdraw.channel_id),
                Op::ChannelInscribe(inscription) => Some(inscription.channel_id),
                _ => None,
            })
            .filter_map(|channel_id| {
                channels
                    .channel_state(&channel_id)
                    .and_then(|channel| channel.note_set)
                    .map(|note_set| (channel_id, note_set))
            })
            .collect();
        WitnessContext {
            channel_note_sets,
            pow_epoch_reward: tip_ledger.mantle_ledger().pow.epoch_reward(),
        }
    }

    // Proves `witness` with the keys of its signers, in input order.
    async fn prove_witness(
        witness: OpWitness,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<ZkTransferProof, WalletServiceError> {
        // Use hex-encoded public key as key_id for now
        let key_ids = witness
            .signers
            .iter()
            .map(|pk| hex::encode(lb_groth16::fr_to_bytes(pk.as_fr())))
            .collect();

        // Pinned to keep the future off the stack: a KMS message can carry a
        // ZkTransfer witness.
        let signature = Box::pin(kms.sign_multiple(key_ids, PayloadEncoding::Zk(witness.witness)))
            .await
            .map_err(WalletServiceError::KmsApi)?;

        let SignatureEncoding::Zk(proof) = signature else {
            return Err(WalletServiceError::KmsApi(
                "Expected ZkTransfer proof".into(),
            ));
        };

        Ok(ZkTransferProof::new(proof))
    }

    // The Ed25519 signature of `op`: by the inscription signer, or by the
    // provider of the declaration.
    async fn op_signature(
        tx_hash: TxHash,
        op: &OpRef<'_>,
        tip_ledger: &LedgerState,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<Option<Ed25519Signature>, WalletServiceError> {
        let signer = match op {
            OpRef::ChannelInscribe(inscribe_op) => {
                let Ok(signer) = Ed25519PublicKey::try_from(inscribe_op.signer) else {
                    return Err(WalletServiceError::InvalidSigner(
                        inscribe_op.signer.to_bytes(),
                    ));
                };
                signer
            }
            OpRef::SDPDeclare(declare_op) => declare_op.provider_id.0,
            OpRef::SDPWithdraw(withdraw_op) => {
                Self::declaration_provider(&withdraw_op.declaration_id, tip_ledger)?
            }
            OpRef::SDPActive(active_op) => {
                Self::declaration_provider(&active_op.declaration_id, tip_ledger)?
            }
            OpRef::ChannelConfig(_)
            | OpRef::ChannelDeposit(_)
            | OpRef::ChannelWithdraw(_)
            | OpRef::Transfer(_)
            | OpRef::ClaimPowReward(_)
            | OpRef::ProgramRegister(_) => return Ok(None),
        };
        Self::sign_ed25519(tx_hash, signer, kms).await.map(Some)
    }

    fn declaration_provider(
        declaration_id: &lb_core::sdp::DeclarationId,
        tip_ledger: &LedgerState,
    ) -> Result<Ed25519PublicKey, WalletServiceError> {
        let declaration = tip_ledger
            .mantle_ledger()
            .sdp_ledger()
            .get_declaration(declaration_id)
            .ok_or(WalletServiceError::MissingDeclaration(*declaration_id))?;
        Ok(declaration.provider_id.0)
    }

    async fn sign_ed25519(
        tx_hash: TxHash,
        pk: <Ed25519Key as SecuredKey>::PublicKey,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<<Ed25519Key as SecuredKey>::Signature, WalletServiceError> {
        // Use hex-encoded public key as key_id for now
        let key_id = hex::encode(pk.as_bytes());

        let payload = PayloadEncoding::Ed25519(Bytes::copy_from_slice(tx_hash.as_signing_bytes()));
        // TODO(RFC-480): pinned to keep the future off the stack, a KMS message
        // can carry a ZkTransfer witness.
        // let signature = kms
        //     .sign(key_id, payload)
        //     .await
        //     .map_err(WalletServiceError::KmsApi)?;
        let signature = Box::pin(kms.sign(key_id, payload))
            .await
            .map_err(WalletServiceError::KmsApi)?;

        let SignatureEncoding::Ed25519(ed25519_sig) = signature else {
            return Err(WalletServiceError::KmsApi(
                "Expected Ed25519 signature".into(),
            ));
        };

        Ok(ed25519_sig)
    }

    // TODO(RFC-480): Operations are proven by ZkTransfers built from the wallet's
    // paths.
    /*
    async fn sign_zksig(
        tx_hash: TxHash,
        pks: impl IntoIterator<Item = ZkPublicKey>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<ZkSignature, WalletServiceError> {
        let pks = ZkPublicKeys::try_from_iter(pks)?;
        // Use hex-encoded public key as key_id for now
        let key_ids: Vec<_> = pks
            .into_iter()
            .map(|pk| hex::encode(lb_groth16::fr_to_bytes(&pk.into_inner())))
            .collect();

        let payload = PayloadEncoding::Zk(tx_hash.to_fr());
        let signature = kms
            .sign_multiple(key_ids, payload)
            .await
            .map_err(WalletServiceError::KmsApi)?;

        let SignatureEncoding::Zk(zk_sig) = signature else {
            return Err(WalletServiceError::KmsApi(
                "Expected ZkSig signature".into(),
            ));
        };

        Ok(zk_sig)
    }

    fn resolve_note_input_pks(
        ledger: &LedgerState,
        note_ids: impl IntoIterator<Item = NoteId>,
    ) -> Result<Vec<ZkPublicKey>, WalletServiceError> {
        note_ids
            .into_iter()
            .map(|note_id| {
                ledger
                    .latest_utxos()
                    .utxos()
                    .get(&note_id)
                    .map(|(utxo, _)| utxo.note.pk)
                    .ok_or(WalletServiceError::MissingInputNote(note_id))
            })
            .collect()
    }

    fn generate_poc(
        voucher_secret: VoucherSecret,
        path: &MerklePath,
        rewards_root: RewardsRoot,
        tx_hash: TxHash,
    ) -> Result<Groth16LeaderClaimProof, WalletServiceError> {
        Ok(Groth16LeaderClaimProof::prove(
            LeaderClaimPrivate::try_new(
                LeaderClaimPublic {
                    voucher_nullifier: VoucherNullifier::from_secret(voucher_secret).into(),
                    voucher_root: rewards_root.into(),
                    mantle_tx_hash: tx_hash.to_fr(),
                },
                path,
                voucher_secret,
            )?,
        )?)
    }
    */

    async fn owned_note_at(
        tip: Option<HeaderId>,
        commitment: &NoteCm,
        wallet: &Wallet,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
    ) -> Result<TipResponse<Option<OwnedNote>>, WalletServiceError> {
        let tip = Self::msg_tip_or_latest(tip, cryptarchia).await?;
        let owned_note = wallet.wallet_state_at(tip)?.notes.get(commitment).cloned();
        Ok(TipResponse {
            tip,
            response: owned_note,
        })
    }

    /// Resolves the wallet-owned UTXOs that are eligible to lead at `tip`
    /// (or at the current tip when `tip` is `None`), paired with the key ids
    /// needed to build a leadership proof for them.
    async fn leader_aged_notes_at(
        tip: Option<HeaderId>,
        wallet: &Wallet,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
    ) -> Result<TipResponse<Vec<AgedNote>>, WalletServiceError> {
        let tip = Self::msg_tip_or_latest(tip, cryptarchia).await?;
        // TODO(RFC-480): the wallet keeps the aged paths of its notes.
        // let ledger_state = Self::ledger_state_at(tip, cryptarchia).await?;

        let wallet_state = wallet.wallet_state_at(tip).map_err(|err| {
            error!(target: LOG_TARGET, err = ?err, "Failed to fetch wallet state");
            WalletServiceError::FailedToFetchWalletStateForBlock(tip)
        })?;

        // TODO(RFC-480): the wallet keeps the aged paths of its notes.
        // let aged_utxos = ledger_state.epoch_state().utxos.utxos();
        // let eligible_utxos = wallet_state
        //     .utxos
        //     .iter()
        //     .filter(|(note_id, _)| aged_utxos.contains_key(note_id))
        //     .filter_map(|(_, utxo)| {
        //         wallet
        //             .known_keys()
        //             .get(&utxo.note.pk)
        //             .map(|key_id| UtxoWithKeyId {
        //                 utxo: *utxo,
        //                 key_id: key_id.clone(),
        //             })
        //     })
        //     .collect();
        let eligible_notes = wallet_state
            .aged_notes()
            .into_iter()
            .filter_map(|(note, aged_path)| {
                wallet
                    .known_keys()
                    .get(&note.note.public_key)
                    .map(|key_id| AgedNote {
                        note,
                        aged_path,
                        key_id: key_id.clone(),
                    })
            })
            .collect();

        Ok(TipResponse {
            tip,
            response: eligible_notes,
        })
    }

    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    /*
    /// Derive a new voucher via KMS and store it in [`Wallet`].
    async fn generate_new_voucher_secret(
        state: &mut ServiceState<'_>,
        master_key_id: KeyId,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        resp_tx: Sender<Result<VoucherCm, WalletServiceError>>,
    ) {
        let index = state.next_new_voucher_index();
        let secret = match Self::derive_voucher_from_kms(kms, master_key_id.clone(), index).await {
            Ok(secret) => secret,
            Err(err) => {
                Self::send_err(resp_tx, err);
                return;
            }
        };
        let cm = VoucherCm::from_secret(secret);
        let nf = VoucherNullifier::from_secret(secret);

        state.add_next_known_voucher(cm, nf, master_key_id);

        if let Err(e) = resp_tx.send(Ok(cm)) {
            debug!(target: LOG_TARGET, "Failed to send voucher secret: {e:?}");
        }
    }
    */

    // Derives the reward key `index` from the voucher master key and makes it
    // a key of the KMS and of the wallet.
    async fn register_reward_key(
        state: &mut ServiceState<'_>,
        master_key_id: KeyId,
        index: u64,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<ZkPublicKey, WalletServiceError> {
        let key = ZkKey::new(Self::derive_voucher_from_kms(kms, master_key_id, index).await?);
        let pk = key.to_public_key();
        let key_id = hex::encode(lb_groth16::fr_to_bytes(pk.as_fr()));
        // Pinned to keep the futures off the stack: a KMS message can carry a
        // ZkTransfer witness.
        if let Err(error) = Box::pin(kms.register(key_id.clone(), Key::Zk(key))).await {
            // The KMS refuses a key id it already holds: the key of this
            // index, registered before.
            Box::pin(kms.public_key(key_id.clone()))
                .await
                .map_err(|_| WalletServiceError::KmsApi(error))?;
        }
        state.add_known_key(pk, key_id);
        Ok(pk)
    }

    // The opening of `note` with the nullifier its key derives.
    async fn known_note(
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        key_id: KeyId,
        note: Note,
        note_set: NoteSetIndex,
    ) -> Result<KnownNote, WalletServiceError> {
        let (output_tx, output_rx) = oneshot::channel();
        // Pinned to keep the future off the stack: a KMS message can carry a
        // ZkTransfer witness.
        Box::pin(kms.execute(
            key_id,
            KeyOperators::Zk(Box::new(DeriveNullifier::new(note.commitment(), output_tx))),
        ))
        .await
        .map_err(|error| WalletServiceError::KmsApi(Box::new(error)))?;
        let nullifier = output_rx.await.map_err(|_| {
            WalletServiceError::KmsApi("KMS API did not respond with the nullifier".into())
        })?;
        Ok(KnownNote {
            note,
            note_set,
            nullifier,
        })
    }

    async fn import_note(
        note: Note,
        note_set: NoteSetIndex,
        state: &mut ServiceState<'_>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<(), WalletServiceError> {
        let key_id = state
            .wallet()
            .known_keys()
            .get(&note.public_key)
            .cloned()
            .ok_or(WalletServiceError::UnknownKey(note.public_key))?;
        let known_note = Self::known_note(kms, key_id, note, note_set).await?;
        state.add_known_notes([known_note]);
        Ok(())
    }

    // Registers the change note `funded` returns to a key of the wallet.
    async fn add_change_note(
        funded: &MantleTxBuilder,
        change_pk: ZkPublicKey,
        change_nonce: Fr,
        state: &mut ServiceState<'_>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<(), WalletServiceError> {
        let Some(key_id) = state.wallet().known_keys().get(&change_pk).cloned() else {
            return Ok(());
        };
        let change = funded
            .openings()
            .into_values()
            .flat_map(|opening| opening.outputs)
            .find(|note| note.nonce == change_nonce && note.public_key == change_pk);
        if let Some(change) = change {
            let known_note = Self::known_note(kms, key_id, change, LEDGER_SET).await?;
            state.add_known_notes([known_note]);
        }
        Ok(())
    }

    // Pushes a Transfer giving `owner` a note of exactly `value` and the rest
    // back to `owner`, registers both notes and returns the nullifiers it
    // spends.
    async fn prepare_exact_note(
        tip: HeaderId,
        tx_builder: MantleTxBuilder,
        value: Value,
        owner: ZkPublicKey,
        funding_pks: Vec<ZkPublicKey>,
        state: &mut ServiceState<'_>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<(MantleTxBuilder, KnownNote, Vec<NoteNf>), WalletServiceError> {
        let key_id = state
            .wallet()
            .known_keys()
            .get(&owner)
            .cloned()
            .ok_or(WalletServiceError::UnknownKey(owner))?;
        let nonce = random_nonce();
        let (split, opening) = state.exact_note_split(
            tip,
            &tx_builder,
            value,
            owner,
            nonce,
            random_nonce(),
            funding_pks,
        )?;
        let nullifiers = split.inputs.iter().copied().collect();

        // the exact note is output 0, the rest output 1
        let exact_note = Self::known_note(
            kms,
            key_id.clone(),
            Note::new(value, nonce, owner),
            LEDGER_SET,
        )
        .await?;
        let mut known_notes = vec![exact_note];
        for note in opening.outputs.iter().skip(1) {
            known_notes.push(Self::known_note(kms, key_id.clone(), *note, LEDGER_SET).await?);
        }

        let tx_builder = tx_builder.push_op_with_opening(Op::Transfer(split), opening)?;
        state.add_known_notes(known_notes);
        Ok((tx_builder, exact_note, nullifiers))
    }

    // Registers the notes `block` reveals to a key of the wallet, so that
    // applying the block adopts them.
    async fn add_revealed_notes(
        block: &WalletBlock,
        state: &mut ServiceState<'_>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<(), WalletServiceError> {
        for (note_set, note) in &block.revealed {
            let Some(key_id) = state.wallet().known_keys().get(&note.public_key).cloned() else {
                continue;
            };
            if state
                .wallet()
                .known_notes()
                .contains_key(&note.commitment())
            {
                continue;
            }
            let known_note = Self::known_note(kms, key_id, *note, *note_set).await?;
            state.add_known_notes([known_note]);
        }
        Ok(())
    }

    // Initializes the wallet from its persisted state at LIB, or from the
    // genesis block on a fresh start, and registers its reward keys again.
    async fn start_state<'u>(
        mut initial_state: RecoveryState,
        settings: &WalletServiceSettings,
        updater: &'u StateUpdater<Option<RecoveryState>>,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<ServiceState<'u>, WalletServiceError> {
        let mut state = match initial_state.take_lib_wallet_state() {
            Some((lib, wallet_state)) => ServiceState::from_lib_wallet_state(
                initial_state,
                lib,
                wallet_state,
                settings,
                updater,
            ),
            None => Self::genesis_state(initial_state, settings, updater, cryptarchia, kms).await?,
        };

        // The reward keys derived before a restart are registered again.
        for index in 0..state.reward_key_count() {
            Self::register_reward_key(
                &mut state,
                settings.voucher_master_key_id.clone(),
                index,
                kms,
            )
            .await?;
        }
        Ok(state)
    }

    // A fresh wallet starts at the genesis block, knowing the genesis notes
    // of its keys.
    async fn genesis_state<'u>(
        state: RecoveryState,
        settings: &WalletServiceSettings,
        updater: &'u StateUpdater<Option<RecoveryState>>,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<ServiceState<'u>, WalletServiceError> {
        let genesis = cryptarchia.get_genesis_block().await?;
        let genesis_tx = genesis.genesis_tx();
        let notes = genesis_tx
            .distribution()
            .iter()
            .enumerate()
            .map(|(index, allocation)| allocation.note(index))
            .collect::<Vec<_>>();
        let declarations = genesis_tx.clone().into_genesis_ops().declarations;
        let service_notes = declarations
            .iter()
            .map(|declaration| declaration.operation().service_note())
            .collect::<Vec<_>>();
        // The genesis declarations spend their ledger notes.
        let spent = declarations
            .iter()
            .flat_map(|declaration| declaration.operation().inputs.iter().copied())
            .collect::<Vec<NoteNf>>();

        let key_ids = settings
            .known_keys
            .iter()
            .map(|(key_id, pk)| (*pk, key_id))
            .collect::<HashMap<_, _>>();
        let mut known_notes = Vec::new();
        for (note_set, note) in notes
            .iter()
            .map(|note| (LEDGER_SET, note))
            .chain(service_notes.iter().map(|note| (SDP_SET, note)))
        {
            if let Some(key_id) = key_ids.get(&note.public_key) {
                let known_note = Self::known_note(kms, (*key_id).clone(), *note, note_set).await?;
                if !spent.contains(&known_note.nullifier) {
                    known_notes.push(known_note);
                }
            }
        }

        Ok(ServiceState::from_genesis(
            state,
            settings,
            genesis.header().id(),
            &notes,
            &service_notes,
            known_notes,
            updater,
        )?)
    }

    /// Derive voucher secret from KMS given master key and index.
    // TODO: Use secure KMS operator that returns `VoucherCm` and `VoucherNullifier`
    async fn derive_voucher_from_kms(
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        key_id: KeyId,
        index: u64,
        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // ) -> Result<VoucherSecret, WalletServiceError> {
    ) -> Result<Fr, WalletServiceError> {
        let (output_tx, output_rx) = oneshot::channel();
        // Pinned to keep the future off the stack: a KMS message can carry a
        // ZkTransfer witness.
        Box::pin(kms.execute(
            key_id,
            KeyOperators::Zk(Box::new(UnsafeVoucherOperator::new(
                index.into(),
                output_tx,
            ))),
        ))
        .await
        .map_err(|error| WalletServiceError::KmsApi(Box::new(error)))?;

        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // Ok(output_rx
        //     .await
        //     .map_err(|_| {
        //         WalletServiceError::KmsApi("KMS API did not respond with voucher_cm".into())
        //     })?
        //     .into())
        output_rx.await.map_err(|_| {
            WalletServiceError::KmsApi("KMS API did not respond with voucher_cm".into())
        })
    }

    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    /*
    async fn build_leader_claim_tx(
        request: LeaderClaimTxRequest,
        ledger: LedgerState,
        state: &mut ServiceState<'_>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<LeaderClaimTx, WalletServiceError> {
        let voucher_nullifier = Self::reserve_claimable_voucher(state, request.tip)?
            .ok_or(WalletServiceError::NoClaimableVoucher)?;

        let result =
            Self::build_reserved_leader_claim_tx(request, voucher_nullifier, ledger, state, kms)
                .await;

        if result.is_err() {
            state.release_claim_reservation(voucher_nullifier);
        }

        result.map(|(signed_tx, funded_notes)| LeaderClaimTx {
            signed_tx,
            voucher_nullifier,
            funded_notes,
        })
    }

    fn reserve_claimable_voucher(
        state: &mut ServiceState<'_>,
        tip: HeaderId,
    ) -> Result<Option<VoucherNullifier>, WalletServiceError> {
        let claimable_vouchers = state.claimable_vouchers(tip)?;

        match (
            claimable_vouchers.available.into_iter().next(),
            claimable_vouchers.pending.len(),
        ) {
            (Some(voucher), _) => {
                state.reserve_claim(voucher.nullifier);

                debug!(
                    target: LOG_TARGET,
                    nf = ?voucher.nullifier,
                    cm = ?voucher.commitment,
                    "Found and reserved claimable voucher"
                );

                Ok(Some(voucher.nullifier))
            }
            (None, pending_count) if pending_count > 0 => {
                debug!(
                    target: LOG_TARGET,
                    "No available vouchers: {pending_count} are pending"
                );
                Ok(None)
            }
            (None, _) => Ok(None),
        }
    }

    async fn get_claimable_vouchers(
        tip: Option<HeaderId>,
        resp_tx: Sender<Result<TipResponse<ClaimableVouchersInfo>, WalletServiceError>>,
        state: &ServiceState<'_>,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
    ) {
        let tip = match Self::msg_tip_or_latest(tip, cryptarchia).await {
            Ok(tip) => tip,
            Err(err) => {
                Self::send_err(resp_tx, err);
                return;
            }
        };

        // The per-voucher payout is a property of the ledger at `tip`, not of
        // the wallet, and it is the same figure the leader claim transaction
        // is built with.
        let reward_amount = match Self::ledger_state_at(tip, cryptarchia).await {
            Ok(ledger_state) => ledger_state.mantle_ledger().leader_reward_amount(),
            Err(err) => {
                Self::send_err(resp_tx, err);
                return;
            }
        };

        let response = state.claimable_vouchers(tip).map(|vouchers| TipResponse {
            tip,
            response: ClaimableVouchersInfo::new(
                vouchers
                    .available
                    .into_iter()
                    .map(|voucher| ClaimableVoucherInfo {
                        commitment: voucher.commitment,
                        nullifier: voucher.nullifier,
                    })
                    .collect(),
                reward_amount,
            ),
        });

        if resp_tx.send(response).is_err() {
            debug!(target: LOG_TARGET, "Failed to respond to GetClaimableVouchers");
        }
    }

    async fn build_reserved_leader_claim_tx(
        request: LeaderClaimTxRequest,
        voucher_nullifier: VoucherNullifier,
        ledger: LedgerState,
        state: &mut ServiceState<'_>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<(SignedOps<Preverified, StandardMode>, Vec<NoteId>), WalletServiceError> {
        let context = ledger.tx_context();
        let tx_builder = MantleTxBuilder::new().push_op(Op::LeaderClaim(LeaderClaimOp {
            rewards_root: request.rewards_root,
            voucher_nullifier,
            pk: request.funding_pk,
        }))?;

        let funded_tx_builder = state.fund_tx::<MainnetGasProfile>(
            request.tip,
            &tx_builder,
            request.funding_pk,
            [request.funding_pk],
            &context,
            0,
        )?;

        let funded_notes: Vec<NoteId> = funded_tx_builder
            .notes_consumed_or_used_in_service()
            .collect();
        state.reserve_pending_notes(funded_notes.iter().copied());

        match Self::sign_funded_leader_claim_tx(request, funded_tx_builder, ledger, state, kms)
            .await
        {
            Ok(signed_tx) => Ok((signed_tx, funded_notes)),
            Err(err) => {
                state.release_pending_notes(funded_notes);
                Err(err)
            }
        }
    }

    async fn sign_funded_leader_claim_tx(
        request: LeaderClaimTxRequest,
        funded_tx_builder: MantleTxBuilder,
        ledger: LedgerState,
        state: &ServiceState<'_>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
    ) -> Result<SignedOps<Preverified, StandardMode>, WalletServiceError> {
        let context = ledger.tx_context();
        let net_balance = funded_tx_builder.net_balance();
        let gas_cost = funded_tx_builder.minimum_gas_cost::<MainnetGasProfile>(&context)?;
        debug!(
            target: LOG_TARGET,
            net_balance,
            gas_cost = ?gas_cost,
            reward_amount = request.reward_amount,
            n_inputs = funded_tx_builder.ledger_inputs().len(),
            "leader claim tx builder state after funding"
        );

        let tx_fee = funded_tx_builder.tx_fee()?;
        if tx_fee > request.max_tx_fee {
            return Err(WalletServiceError::TxFeeExceedsMaxFee {
                max_fee: request.max_tx_fee,
                tx_fee,
            });
        }

        Self::sign_tx(funded_tx_builder, request.tip, ledger, kms, state.wallet()).await
    }
    */

    async fn backfill_if_not_in_sync(
        tip: Option<HeaderId>,
        state: &mut ServiceState<'_>,
        storage: &StorageApi<Tx>,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        epoch_config: &EpochConfig,
    ) -> Result<(), WalletServiceError> {
        let tip = Self::msg_tip_or_latest(tip, cryptarchia).await?;

        if state.wallet().has_processed_block(tip) {
            // We are already in sync with `tip`.
            return Ok(());
        }

        // The caller knows a more recent tip than the wallet.
        // To resolve this, we do a JIT backfill to try to sync the wallet with
        // cryptarchia. If we still have not caught up after the backfill, we return an
        // error to the caller
        Self::backfill_missing_blocks(tip, state, storage, cryptarchia, kms, epoch_config).await?;

        if state.wallet().has_processed_block(tip) {
            Ok(())
        } else {
            error!(target: LOG_TARGET, "Failed to backfill wallet to {tip}");
            Err(WalletServiceError::FailedToFetchWalletStateForBlock(tip))
        }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: address this in a dedicated refactor"
    )]
    async fn handle_new_block(
        header_id: HeaderId,
        state: &mut ServiceState<'_>,
        storage: &StorageApi<Tx>,
        cryptarchia_api: &CryptarchiaServiceApi<Cryptarchia>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        epoch_config: &EpochConfig,
    ) {
        let Ok(block) = Self::load_block(header_id, storage).await.inspect_err(|e| {
            error!(
                target: LOG_TARGET,
                block_id = ?header_id,
                err = %e,
                "Failed to fetch new block and ledger for wallet"
            );
        }) else {
            return;
        };

        let events = Self::load_block_events(header_id, storage).await;
        let wallet_block =
            WalletBlock::from_block(&block, epoch_config.epoch(block.header().slot()), &events);
        // An owned note missed here is never adopted, so the block is left
        // unapplied and is backfilled from the next one.
        if let Err(e) = Self::add_revealed_notes(&wallet_block, state, kms).await {
            error!(
                target: LOG_TARGET,
                block_id = ?header_id,
                err = %e,
                "Failed to register the notes the block reveals to the wallet"
            );
            return;
        }
        match state.apply_block(&wallet_block) {
            Ok(()) => {
                trace!(target: LOG_TARGET, block_id = ?wallet_block.id, "Applied block to wallet");
            }
            Err(WalletError::UnknownBlock(block_id)) => {
                debug!(
                    target: LOG_TARGET,
                    block_id = ?block_id,
                    "Missing block in wallet, backfilling"
                );
                if let Err(e) = Self::backfill_missing_blocks(
                    wallet_block.id,
                    state,
                    storage,
                    cryptarchia_api,
                    kms,
                    epoch_config,
                )
                .await
                {
                    error!(
                        target: LOG_TARGET,
                        block_id = ?header_id,
                        err = %e,
                        "Failed to backfill missing block to wallet"
                    );
                }
            }
            Err(e) => {
                error!(
                    target: LOG_TARGET,
                    err = %e,
                    "unexexpected error while applying block to wallet"
                );
            }
        }
    }

    async fn load_block(
        header_id: HeaderId,
        storage: &StorageApi<Tx>,
    ) -> Result<Block<Tx>, WalletServiceError> {
        storage
            .get_block(&header_id)
            .await
            .ok_or(WalletServiceError::BlockNotFoundInStorage(header_id))
    }

    async fn load_block_events(header_id: HeaderId, storage: &StorageApi<Tx>) -> Events {
        storage
            .get_block_events(&header_id)
            .await
            .unwrap_or_else(|| {
                warn!(
                    target: LOG_TARGET,
                    block_id = ?header_id,
                    "Failed to load block events for wallet"
                );
                Events::new()
            })
    }

    async fn handle_lib_update(
        lib_update: &LibUpdate,
        storage: &StorageApi<Tx>,
        state: &mut ServiceState<'_>,
        cryptarchia_api: &CryptarchiaServiceApi<Cryptarchia>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        epoch_config: &EpochConfig,
    ) {
        log_lib_update(lib_update);

        // The wallet's frontier can lag the chain's LIB: during bootstrap the
        // engine jumps the LIB (e.g. on switch-to-online) without broadcasting
        // intermediate updates, so `new_lib` may be a block the wallet never
        // applied. Backfill up to it first, mirroring `handle_new_block`, so the
        // subsequent `advance_lib` finds the `WalletState` it needs.
        if !state.wallet().has_processed_block(lib_update.new_lib) {
            debug!(
                target: LOG_TARGET,
                new_lib = ?lib_update.new_lib,
                "Wallet missing new LIB block, backfilling before advancing LIB"
            );
            if let Err(e) = Self::backfill_missing_blocks(
                lib_update.new_lib,
                state,
                storage,
                cryptarchia_api,
                kms,
                epoch_config,
            )
            .await
            {
                error!(
                    target: LOG_TARGET,
                    new_lib = ?lib_update.new_lib,
                    err = %e,
                    "Failed to backfill wallet to new LIB; skipping LIB advance"
                );
                return;
            }
        }

        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // let claimed_nullifiers = Self::collect_claimed_nullifiers_from_blocks(
        //     lib_update.pruned_blocks.immutable_blocks.values(),
        //     storage,
        // )
        // .await;
        //
        // for nullifier in &claimed_nullifiers {
        //     state.release_claim_reservation(*nullifier);
        // }

        let new_immutable_blocks_count = lib_update.pruned_blocks.immutable_blocks.len() as u64;
        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // state.advance_lib(
        //     lib_update.new_lib,
        //     lib_update.pruned_blocks.all(),
        //     new_immutable_blocks_count,
        //     claimed_nullifiers,
        // );
        state.advance_lib(
            lib_update.new_lib,
            lib_update.pruned_blocks.all(),
            new_immutable_blocks_count,
        );
    }

    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    /*
    async fn collect_claimed_nullifiers_from_blocks(
        blocks: impl Iterator<Item = &HeaderId>,
        storage: &StorageApi<Tx>,
    ) -> Vec<VoucherNullifier> {
        let immutable_blocks: Vec<Block<Tx>> = futures::stream::iter(blocks)
            .filter_map(async |header_id| storage.get_block(header_id).await)
            .collect::<Vec<_>>()
            .await;

        immutable_blocks
            .into_iter()
            .flat_map(|block: Block<Tx>| block.into_transactions().into_iter())
            .collect::<Vec<_>>() // Need to collect to avoid lifetime issues
            .iter()
            .flat_map(MantleTx::op_refs)
            .filter_map(|op| {
                if let OpRef::LeaderClaim(claim_op) = op {
                    Some(claim_op.voucher_nullifier)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    }
    */

    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: address this in a dedicated refactor"
    )]
    async fn backfill_missing_blocks(
        tip: HeaderId,
        state: &mut ServiceState<'_>,
        storage: &StorageApi<Tx>,
        cryptarchia_api: &CryptarchiaServiceApi<Cryptarchia>,
        kms: &KmsServiceApi<Kms, RuntimeServiceId>,
        epoch_config: &EpochConfig,
    ) -> Result<(), WalletServiceError> {
        debug!(
            target: LOG_TARGET,
            from_tip = ?tip,
            to_state_lib = ?state.lib(),
            "backfilling missing blocks"
        );

        // Fetch block IDs in [state.lib, tip]
        let missing_headers = cryptarchia_api
            .get_headers(Some(tip), Some(state.lib()))
            .await
            .map_err(WalletServiceError::CryptarchiaApi)
            .inspect_err(|e| {
                error!(
                    target: LOG_TARGET,
                    block_id = ?tip,
                    err = %e,
                    "Failed to fetch missing headers for backfill"
                );
            })?
            .try_collect::<Vec<_>>()
            .await?;

        if !missing_headers.is_empty() {
            debug!(
                target: LOG_TARGET,
                "Backfilling wallet to tip {tip:?} with {} missing headers",
                missing_headers.len()
            );
        }

        // Load/apply blocks in order from `state.lib` to `tip`
        for header_id in missing_headers.into_iter().rev() {
            if state.wallet().has_processed_block(header_id) {
                debug!(
                    target: LOG_TARGET,
                    "Skipping already processed wallet block {header_id:?}"
                );
                continue;
            }

            let block = Self::load_block(header_id, storage).await?;
            let events = Self::load_block_events(header_id, storage).await;
            let wallet_block =
                WalletBlock::from_block(&block, epoch_config.epoch(block.header().slot()), &events);
            Self::add_revealed_notes(&wallet_block, state, kms).await?;

            if let Err(e) = state.apply_block(&wallet_block) {
                error!(
                    target: LOG_TARGET,
                    block_id = ?header_id,
                    err = %e,
                    "Failed to apply backfill block to wallet"
                );
                return Err(WalletServiceError::FailedToApplyBlock(header_id));
            }
        }

        Ok(())
    }

    fn send_err<T: std::fmt::Debug>(
        tx: Sender<Result<T, WalletServiceError>>,
        err: WalletServiceError,
    ) {
        if let Err(msg) = tx.send(Err(err)) {
            debug!(target: LOG_TARGET, msg = ?msg, "Wallet failed to send error response");
        }
    }

    fn get_known_addresses(
        wallet: &Wallet,
        tx: Sender<Result<Vec<ZkPublicKey>, WalletServiceError>>,
    ) {
        let response: Vec<_> = wallet.known_keys().keys().copied().collect();
        if let Err(e) = tx.send(Ok(response)) {
            debug!(target: LOG_TARGET, err = ?e, "Failed to send known addresses response");
        }
    }

    async fn get_tx_context(
        block_id: Option<HeaderId>,
        resp_tx: Sender<Result<OpsContext, WalletServiceError>>,
        cryptarchia: &CryptarchiaServiceApi<Cryptarchia>,
    ) {
        let block_id = match Self::msg_tip_or_latest(block_id, cryptarchia).await {
            Ok(block_id) => block_id,
            Err(error) => {
                Self::send_err(resp_tx, error);
                return;
            }
        };

        let ledger_state = match Self::ledger_state_at(block_id, cryptarchia).await {
            Ok(ledger_state) => ledger_state,
            Err(err) => {
                Self::send_err(resp_tx, err);
                return;
            }
        };

        if let Err(e) = resp_tx.send(Ok(ledger_state.tx_context())) {
            debug!(target: LOG_TARGET, err = ?e, "Failed to send gas context response");
        }
    }
}

fn log_lib_update(lib_update: &LibUpdate) {
    let stale_blocks_count = lib_update.pruned_blocks.stale_blocks.len();
    let immutable_blocks_count = lib_update.pruned_blocks.immutable_blocks.len();
    if stale_blocks_count == 0 && immutable_blocks_count == 1 {
        trace!(
            target: LOG_TARGET,
            new_lib = ?lib_update.new_lib,
            stale_blocks_count,
            immutable_blocks_count,
            "Received LIB update"
        );
    } else {
        debug!(
            target: LOG_TARGET,
            new_lib = ?lib_update.new_lib,
            stale_blocks_count,
            immutable_blocks_count,
            "Received LIB update"
        );
    }
}

// A fresh nonce for a note the wallet creates.
fn random_nonce() -> Fr {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    lb_groth16::fr_from_mod_bytes(&bytes)
}

// The proof of each Operation from its ZkTransfer and Ed25519 parts.
const fn op_proof(
    op: &OpRef<'_>,
    zk_proof: Option<ZkTransferProof>,
    signature: Option<Ed25519Signature>,
) -> Result<OpProof, WalletServiceError> {
    match (op, zk_proof, signature) {
        (OpRef::ChannelInscribe(inscription), _, _) if !inscription.steps.is_empty() => {
            Err(WalletServiceError::UnsupportedSteps)
        }
        (OpRef::ChannelInscribe(_), None, Some(signature)) => Ok(OpProof::ChannelInscribe(
            ChannelInscribeProof::without_steps(signature),
        )),
        (
            OpRef::Transfer(_) | OpRef::ChannelDeposit(_) | OpRef::ChannelWithdraw(_),
            Some(zk_proof),
            None,
        ) => Ok(OpProof::ZkTransfer(zk_proof)),
        (OpRef::SDPDeclare(_) | OpRef::SDPWithdraw(_), Some(zk_proof), Some(signature)) => Ok(
            OpProof::ZkAndEd25519Sigs(ZkAndEd25519Proof::new(zk_proof, signature)),
        ),
        (OpRef::SDPActive(_), None, Some(signature)) => Ok(OpProof::Ed25519Sig(signature)),
        (OpRef::ClaimPowReward(_) | OpRef::ProgramRegister(_), None, None) => {
            Ok(OpProof::None(NoOpProof))
        }
        _ => Err(WalletServiceError::OpProofParts),
    }
}

/// A config to calculate epoch from slot
struct EpochConfig {
    epoch_config: lb_cryptarchia_engine::EpochConfig,
    consensus_config: lb_cryptarchia_engine::Config,
}

impl EpochConfig {
    fn epoch(&self, slot: Slot) -> Epoch {
        self.epoch_config
            .epoch(slot, self.consensus_config.base_period_length())
    }
}

#[cfg(test)]
mod tests {
    use lb_core::mantle::ops::{
        sdp::{SDPActiveOp, SDPDeclareOp},
        transfer::TransferOp,
    };
    use lb_groth16::CompressedGroth16Proof;

    use super::*;

    #[test]
    fn op_proof_pairs_each_operation_with_its_proof_type() {
        let zk = ZkTransferProof::new(CompressedGroth16Proof::from_bytes(&[1u8; 128]));
        let signature = Ed25519Signature::from_bytes(&[2u8; 64]);
        let transfer = TransferOp::sample();
        let declare = SDPDeclareOp::sample();
        let active = SDPActiveOp::sample();
        assert_eq!(
            op_proof(&OpRef::Transfer(&transfer), Some(zk), None).unwrap(),
            OpProof::ZkTransfer(zk)
        );
        assert_eq!(
            op_proof(&OpRef::SDPDeclare(&declare), Some(zk), Some(signature)).unwrap(),
            OpProof::ZkAndEd25519Sigs(ZkAndEd25519Proof::new(zk, signature))
        );
        assert_eq!(
            op_proof(&OpRef::SDPActive(&active), None, Some(signature)).unwrap(),
            OpProof::Ed25519Sig(signature)
        );
        assert!(op_proof(&OpRef::Transfer(&transfer), None, None).is_err());
    }
}
