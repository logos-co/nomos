use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
};

// TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
// use lb_core::{
//     header::HeaderId,
//     mantle::{
//         GasProfile, NoteId,
//         ops::leader_claim::{VoucherCm, VoucherNullifier},
//         transactions::{MantleTxBuilder, tx_list::ops::OpsContext},
//     },
// };
use lb_core::{
    header::HeaderId,
    mantle::{
        GasProfile, Note, Value,
        notes::NoteNf,
        ops::transfer::TransferOp,
        transactions::{MantleTxBuilder, builder::ZkTransferOpening, tx_list::ops::OpsContext},
    },
};
use lb_groth16::Fr;
use lb_key_management_system_service::keys::ZkPublicKey;
// TODO(RFC-480): a wallet follows the commitment MMRs from genesis.
// use lb_ledger::LedgerState;
use lb_log_targets::wallet;
// TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
// use lb_wallet::{Voucher, Vouchers, WalletBlock, WalletError, WalletState};
use lb_wallet::{KnownNote, KnownNotes, WalletBlock, WalletError, WalletState};
use overwatch::services::state::StateUpdater;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::{KeyId, WalletServiceError, WalletServiceSettings};

// TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
// type VoucherIndex = u64;
// type VoucherId = (KeyId, VoucherIndex);
// pub type Wallet = lb_wallet::Wallet<KeyId, VoucherId>;
pub type Wallet = lb_wallet::Wallet<KeyId>;

// TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
/*
pub struct ClaimableVouchers {
    pub available: Vec<Voucher>,
    pub pending: Vec<Voucher>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingClaim {
    immutable_blocks_since_reservation: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PendingClaims {
    claims: HashMap<VoucherNullifier, PendingClaim>,
}

impl PendingClaims {
    fn reserve(&mut self, nullifier: VoucherNullifier) {
        debug!(
            target: wallet::SERVICE,
            ?nullifier,
            "Reserved claim voucher"
        );
        self.claims.insert(
            nullifier,
            PendingClaim {
                immutable_blocks_since_reservation: 0,
            },
        );
    }

    fn release(&mut self, nullifier: VoucherNullifier) {
        if self.claims.remove(&nullifier).is_some() {
            debug!(
                target: wallet::SERVICE,
                ?nullifier,
                "Released pending claim reservation"
            );
        }
    }

    fn is_reserved(&self, nullifier: &VoucherNullifier) -> bool {
        self.claims.contains_key(nullifier)
    }

    /// Evict reservations whose LIB-progress age reached the configured limit.
    ///
    /// Each LIB update adds `new_immutable_blocks_count` to every reservation's
    /// `immutable_blocks_since_reservation` counter. A reservation expires once
    /// that counter reaches `max_immutable_blocks_since_reservation`.
    fn evict_expired(
        &mut self,
        new_immutable_blocks_count: u64,
        max_immutable_blocks_since_reservation: u64,
    ) {
        if new_immutable_blocks_count == 0 {
            return;
        }

        let before_count = self.claims.len();

        self.claims.retain(|nullifier, claim| {
            claim.immutable_blocks_since_reservation = claim
                .immutable_blocks_since_reservation
                .saturating_add(new_immutable_blocks_count);

            let expired =
                claim.immutable_blocks_since_reservation >= max_immutable_blocks_since_reservation;

            if expired {
                debug!(
                    target: wallet::SERVICE,
                    ?nullifier,
                    immutable_blocks_since_reservation = claim.immutable_blocks_since_reservation,
                    max_immutable_blocks_since_reservation,
                    "Removing pending claim reservation after LIB progress"
                );
            }

            !expired
        });

        if before_count != self.claims.len() {
            debug!(
                target: wallet::SERVICE,
                "Cleaned {} pending claim reservation(s) after LIB progress",
                before_count - self.claims.len()
            );
        }
    }
}
*/

/// Notes handed out as funding for transactions that have been built but not
/// yet observed on chain. Ephemeral: rebuilt empty on restart; entries are
/// dropped once the note is observed spent in a block, or expire after enough
/// LIB progress (mirroring [`PendingClaims`]).
///
/// Unlike claim reservations, which must hold until the claim is finalised,
/// funded notes are released as soon as they are observed spent in a tip
/// block; eviction only covers transactions that never land. Releasing too
/// early merely reintroduces a conflicting tx, the same failure mode as
/// having no reservation at all.
#[derive(Debug, Default)]
struct PendingNotes {
    notes: HashMap<NoteNf, u64>,
}

impl PendingNotes {
    fn reserve(&mut self, nullifiers: impl IntoIterator<Item = NoteNf>) {
        for nullifier in nullifiers {
            debug!(
                target: wallet::SERVICE,
                ?nullifier,
                "Reserved pending note"
            );
            self.notes.insert(nullifier, 0);
        }
    }

    fn release(&mut self, nullifiers: impl IntoIterator<Item = NoteNf>) {
        for nullifier in nullifiers {
            if self.notes.remove(&nullifier).is_some() {
                debug!(
                    target: wallet::SERVICE,
                    ?nullifier,
                    "Released pending note reservation"
                );
            }
        }
    }

    fn nullifiers(&self) -> HashSet<NoteNf> {
        self.notes.keys().copied().collect()
    }

    /// Evict reservations whose LIB-progress age reached the configured limit.
    ///
    /// Each LIB update adds `new_immutable_blocks_count` to every reservation's
    /// counter. A reservation expires once that counter reaches
    /// `max_immutable_blocks_since_reservation`.
    fn evict_expired(
        &mut self,
        new_immutable_blocks_count: u64,
        max_immutable_blocks_since_reservation: u64,
    ) {
        if new_immutable_blocks_count == 0 {
            return;
        }

        self.notes
            .retain(|nullifier, immutable_blocks_since_reservation| {
                *immutable_blocks_since_reservation =
                    immutable_blocks_since_reservation.saturating_add(new_immutable_blocks_count);

                let expired =
                    *immutable_blocks_since_reservation >= max_immutable_blocks_since_reservation;

                if expired {
                    debug!(
                        target: wallet::SERVICE,
                        ?nullifier,
                        immutable_blocks_since_reservation,
                        max_immutable_blocks_since_reservation,
                        "Removing pending note reservation after LIB progress"
                    );
                }

                !expired
            });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryState {
    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    // next_new_voucher_index: VoucherIndex,
    // vouchers: Vouchers<VoucherId>,
    /// [`WalletState`] at the last known LIB.
    /// `None` on fresh start; populated after the first LIB update.
    lib_wallet_state: Option<(HeaderId, WalletState)>,
    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    // /// Voucher reservations for claim transactions that were built/submitted
    // /// but have not reached LIB yet. Stale reservations are bounded by
    // /// LIB-progress expiry after recovery.
    // pending_claims: PendingClaims,
    // openings the wallet adopts once their commitments are appended
    known_notes: KnownNotes,
    // the reward keys `0..next_reward_key_index` are derived again at start
    next_reward_key_index: u64,
}

impl RecoveryState {
    pub const fn take_lib_wallet_state(&mut self) -> Option<(HeaderId, WalletState)> {
        self.lib_wallet_state.take()
    }
}

impl overwatch::services::state::ServiceState for RecoveryState {
    type Settings = WalletServiceSettings;
    type Error = WalletServiceError;

    fn from_settings(_settings: &Self::Settings) -> Result<Self, Self::Error> {
        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // Ok(Self {
        //     next_new_voucher_index: 0,
        //     vouchers: Vouchers::default(),
        //     lib_wallet_state: None,
        //     pending_claims: PendingClaims::default(),
        // })
        Ok(Self {
            lib_wallet_state: None,
            known_notes: KnownNotes::new(),
            next_reward_key_index: 0,
        })
    }
}

/// Provides operations on the states that must be synced to [`RecoveryState`].
pub struct ServiceState<'u> {
    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    // next_new_voucher_index: VoucherIndex,
    next_reward_key_index: u64,
    wallet: Wallet,
    lib: HeaderId,
    updater: &'u StateUpdater<Option<RecoveryState>>,
    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    // pending_claims: PendingClaims,
    pending_notes: PendingNotes,
    pending_note_expiry_blocks: u64,
    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    // security_param: u64,
}

impl<'u> ServiceState<'u> {
    // Restores the wallet persisted at its LIB.
    pub fn from_lib_wallet_state(
        state: RecoveryState,
        lib: HeaderId,
        wallet_state: WalletState,
        settings: &WalletServiceSettings,
        updater: &'u StateUpdater<Option<RecoveryState>>,
    ) -> Self {
        let RecoveryState {
            known_notes,
            next_reward_key_index,
            ..
        } = state;
        let wallet =
            Wallet::from_lib_wallet_state(known_keys(settings), known_notes, lib, wallet_state);
        Self::with_wallet(wallet, lib, next_reward_key_index, settings, updater)
    }

    // A fresh wallet follows the note sets from the genesis block.
    pub fn from_genesis(
        state: RecoveryState,
        settings: &WalletServiceSettings,
        genesis: HeaderId,
        genesis_notes: &[Note],
        genesis_service_notes: &[Note],
        genesis_known_notes: impl IntoIterator<Item = KnownNote>,
        updater: &'u StateUpdater<Option<RecoveryState>>,
    ) -> Result<Self, WalletError> {
        let RecoveryState {
            mut known_notes,
            next_reward_key_index,
            ..
        } = state;
        known_notes.extend(
            genesis_known_notes
                .into_iter()
                .map(|known_note| (known_note.note.commitment(), known_note)),
        );
        let wallet = Wallet::from_genesis(
            known_keys(settings),
            known_notes,
            genesis,
            genesis_notes,
            genesis_service_notes,
        )?;
        Ok(Self::with_wallet(
            wallet,
            genesis,
            next_reward_key_index,
            settings,
            updater,
        ))
    }

    fn with_wallet(
        wallet: Wallet,
        lib: HeaderId,
        next_reward_key_index: u64,
        settings: &WalletServiceSettings,
        updater: &'u StateUpdater<Option<RecoveryState>>,
    ) -> Self {
        Self {
            next_reward_key_index,
            wallet,
            lib,
            updater,
            pending_notes: PendingNotes::default(),
            pending_note_expiry_blocks: settings.pending_note_expiry_blocks,
        }
    }

    // TODO(RFC-480): a wallet follows the commitment MMRs from genesis.
    /*
    pub fn new(
        state: RecoveryState,
        settings: &WalletServiceSettings,
        lib: HeaderId,
        lib_ledger: &LedgerState,
        updater: &'u StateUpdater<Option<RecoveryState>>,
        security_param: u64,
    ) -> Self {
        let RecoveryState {
            next_new_voucher_index,
            vouchers,
            lib_wallet_state,
            pending_claims,
        } = state;
        let known_keys = settings
            .known_keys
            .iter()
            .map(|(key_id, pk)| (*pk, key_id.clone()));

        // Initialize [`Wallet`] either from the persisted [`WalletState`]
        // or from the current chain's LIB ledger state.
        let (wallet, wallet_lib) = match lib_wallet_state {
            Some((persisted_lib, wallet_state)) => (
                Wallet::from_lib_wallet_state(known_keys, vouchers, persisted_lib, wallet_state),
                persisted_lib,
            ),
            None => (
                Wallet::from_lib_ledger_state(known_keys, vouchers, lib, lib_ledger),
                lib,
            ),
        };

        Self {
            next_new_voucher_index,
            wallet,
            lib: wallet_lib,
            updater,
            pending_claims,
            pending_notes: PendingNotes::default(),
            pending_note_expiry_blocks: settings.pending_note_expiry_blocks,
            security_param,
        }
    }
    */

    pub const fn lib(&self) -> HeaderId {
        self.lib
    }

    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    /*
    pub const fn next_new_voucher_index(&self) -> VoucherIndex {
        self.next_new_voucher_index
    }

    pub fn add_next_known_voucher(&mut self, cm: VoucherCm, nf: VoucherNullifier, key_id: KeyId) {
        let id = (key_id, self.next_new_voucher_index);
        self.next_new_voucher_index += 1;
        self.wallet.add_known_voucher(cm, nf, id);
        self.update_state();
    }
    */

    // the number of reward keys derived so far
    pub const fn reward_key_count(&self) -> u64 {
        self.next_reward_key_index
    }

    pub fn next_reward_key_index(&mut self) -> u64 {
        let index = self.next_reward_key_index;
        self.next_reward_key_index = index.saturating_add(1);
        self.update_state();
        index
    }

    pub fn add_known_key(&mut self, pk: ZkPublicKey, key_id: KeyId) {
        self.wallet.add_known_key(pk, key_id);
    }

    pub fn add_known_notes(&mut self, notes: impl IntoIterator<Item = KnownNote>) {
        self.wallet.add_known_notes(notes);
        self.update_state();
    }

    pub fn apply_block(&mut self, block: &WalletBlock) -> Result<(), WalletError> {
        self.wallet.apply_block(block)?;
        // Note reservations are released after `pending_note_expiry_blocks`
        // (see `evict_expired`), not on spent.
        self.update_state();
        Ok(())
    }

    pub fn advance_lib(
        &mut self,
        new_lib: HeaderId,
        pruned_blocks: impl IntoIterator<Item = HeaderId>,
        new_immutable_blocks_count: u64,
        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // pruned_nullifiers: impl IntoIterator<Item = VoucherNullifier>,
    ) {
        // The wallet must already hold a `WalletState` for `new_lib`, otherwise
        // `update_state` cannot persist the recovery state. The service backfills
        // up to `new_lib` before calling us; if it still isn't present (e.g. the
        // block is not yet in storage) we skip advancing rather than panicking,
        // keeping `self.lib` pointing at a block whose state exists. A later
        // block application or LIB update advances the LIB once the gap is filled.
        if !self.wallet.has_processed_block(new_lib) {
            warn!(
                target: wallet::SERVICE,
                ?new_lib,
                current_lib = ?self.lib,
                "Skipping LIB advance: wallet has not applied the new LIB block yet"
            );
            return;
        }

        self.lib = new_lib;
        self.wallet.prune_states(pruned_blocks);
        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // self.wallet.prune_vouchers(pruned_nullifiers);
        // self.pending_claims
        //     .evict_expired(new_immutable_blocks_count, self.security_param);
        self.pending_notes
            .evict_expired(new_immutable_blocks_count, self.pending_note_expiry_blocks);
        self.update_state();
    }

    pub const fn wallet(&self) -> &Wallet {
        &self.wallet
    }

    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    /*
    pub fn claimable_vouchers(
        &self,
        tip: HeaderId,
    ) -> Result<ClaimableVouchers, WalletServiceError> {
        let mut available = Vec::new();
        let mut pending = Vec::new();

        for voucher in self.wallet.voucher_commitments_and_nullifiers() {
            if self.pending_claims.is_reserved(&voucher.nullifier) {
                pending.push(voucher);
                continue;
            }

            if self
                .wallet
                .voucher_path_snapshot(tip, &voucher.commitment)?
                .is_some()
            {
                available.push(voucher);
            }
        }

        Ok(ClaimableVouchers { available, pending })
    }

    pub fn reserve_claim(&mut self, nullifier: VoucherNullifier) {
        self.pending_claims.reserve(nullifier);
    }

    pub fn release_claim_reservation(&mut self, nullifier: VoucherNullifier) {
        self.pending_claims.release(nullifier);
    }
    */

    /// Fund `tx_builder` from the wallet's notes at `tip`, excluding notes
    /// already reserved for in-flight transactions. `priority_fee_percent`
    /// reserves that percentage of the final mandatory fee, including both
    /// execution and storage cost. Only the unused reserve becomes an
    /// effective priority tip, and the percentage is not capped at 100.
    #[expect(
        clippy::too_many_arguments,
        reason = "thin passthrough to `Wallet::fund_tx` plus the reservations"
    )]
    pub fn fund_tx<G: GasProfile>(
        &self,
        tip: HeaderId,
        tx_builder: &MantleTxBuilder,
        change_pk: ZkPublicKey,
        change_nonce: Fr,
        funding_pks: impl IntoIterator<Item = impl Borrow<ZkPublicKey>>,
        context: &OpsContext,
        priority_fee_percent: u64,
    ) -> Result<MantleTxBuilder, WalletError> {
        self.wallet.fund_tx::<G>(
            tip,
            tx_builder,
            change_pk,
            change_nonce,
            funding_pks,
            context,
            &self.pending_notes.nullifiers(),
            priority_fee_percent,
        )
    }

    // A Transfer giving `owner` a note of exactly `value` and the rest back to
    // `owner`, from the wallet's notes at `tip` that neither `tx_builder` nor
    // an in flight transaction spends.
    #[expect(
        clippy::too_many_arguments,
        reason = "thin passthrough to `WalletState::exact_note_split` plus the reservations"
    )]
    pub fn exact_note_split(
        &self,
        tip: HeaderId,
        tx_builder: &MantleTxBuilder,
        value: Value,
        owner: ZkPublicKey,
        nonce: Fr,
        change_nonce: Fr,
        funding_pks: impl IntoIterator<Item = impl Borrow<ZkPublicKey>>,
    ) -> Result<(TransferOp, ZkTransferOpening), WalletError> {
        let excluded = tx_builder
            .nullifiers_consumed()
            .chain(self.pending_notes.nullifiers())
            .collect::<HashSet<_>>();
        self.wallet.wallet_state_at(tip)?.exact_note_split(
            value,
            owner,
            nonce,
            owner,
            change_nonce,
            funding_pks,
            &excluded,
        )
    }

    pub fn reserve_pending_notes(&mut self, nullifiers: impl IntoIterator<Item = NoteNf>) {
        self.pending_notes.reserve(nullifiers);
    }

    pub fn release_pending_notes(&mut self, nullifiers: impl IntoIterator<Item = NoteNf>) {
        self.pending_notes.release(nullifiers);
    }

    fn update_state(&self) {
        let lib_wallet_state = self
            .wallet()
            .wallet_state_at(self.lib)
            .expect("WalletState at LIB must exist");

        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // self.updater.update(Some(RecoveryState {
        //     next_new_voucher_index: self.next_new_voucher_index,
        //     vouchers: self.wallet.vouchers().clone(),
        //     lib_wallet_state: Some((self.lib, lib_wallet_state)),
        //     pending_claims: self.pending_claims.clone(),
        // }));
        self.updater.update(Some(RecoveryState {
            lib_wallet_state: Some((self.lib, lib_wallet_state)),
            known_notes: self.wallet.known_notes().clone(),
            next_reward_key_index: self.next_reward_key_index,
        }));
    }
}

fn known_keys(settings: &WalletServiceSettings) -> impl Iterator<Item = (ZkPublicKey, KeyId)> {
    settings
        .known_keys
        .iter()
        .map(|(key_id, pk)| (*pk, key_id.clone()))
}

#[cfg(test)]
mod tests {
    use lb_core::mantle::note_set::LEDGER_SET;
    use lb_groth16::Field as _;

    use super::*;

    const EXPIRY_BLOCKS: u64 = 4;

    #[test]
    fn recovery_state_keeps_the_known_notes_and_the_reward_key_index() {
        let note = KnownNote {
            note: Note::new(10, Fr::ONE, ZkPublicKey::new(Fr::from(2u64))),
            note_set: LEDGER_SET,
            nullifier: NoteNf(Fr::from(3u64)),
        };
        let state = RecoveryState {
            lib_wallet_state: None,
            known_notes: HashMap::from([(note.note.commitment(), note)]),
            next_reward_key_index: 4,
        };
        let decoded: RecoveryState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(decoded.known_notes, state.known_notes);
        assert_eq!(decoded.next_reward_key_index, 4);
    }

    // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
    /*
    #[test]
    fn pending_claim_does_not_expire_without_lib_progress() {
        let nullifier = VoucherNullifier::default();
        let mut pending_claims = PendingClaims::default();

        pending_claims.reserve(nullifier);
        pending_claims.evict_expired(0, EXPIRY_BLOCKS);

        assert!(pending_claims.is_reserved(&nullifier));
    }

    #[test]
    fn pending_claim_expires_after_enough_lib_progress() {
        let nullifier = VoucherNullifier::default();
        let mut pending_claims = PendingClaims::default();

        pending_claims.reserve(nullifier);
        pending_claims.evict_expired(EXPIRY_BLOCKS - 1, EXPIRY_BLOCKS);
        assert!(pending_claims.is_reserved(&nullifier));

        pending_claims.evict_expired(1, EXPIRY_BLOCKS);
        assert!(!pending_claims.is_reserved(&nullifier));
    }
    */

    #[test]
    fn pending_note_does_not_expire_without_lib_progress() {
        let nullifier = NoteNf(Fr::ONE);
        let mut pending_notes = PendingNotes::default();

        pending_notes.reserve([nullifier]);
        pending_notes.evict_expired(0, EXPIRY_BLOCKS);

        assert!(pending_notes.nullifiers().contains(&nullifier));
    }

    #[test]
    fn pending_note_expires_after_enough_lib_progress() {
        let nullifier = NoteNf(Fr::ONE);
        let mut pending_notes = PendingNotes::default();

        pending_notes.reserve([nullifier]);
        pending_notes.evict_expired(EXPIRY_BLOCKS - 1, EXPIRY_BLOCKS);
        assert!(pending_notes.nullifiers().contains(&nullifier));

        pending_notes.evict_expired(1, EXPIRY_BLOCKS);
        assert!(!pending_notes.nullifiers().contains(&nullifier));
    }

    #[test]
    fn released_pending_note_becomes_available() {
        let nullifier = NoteNf(Fr::ONE);
        let mut pending_notes = PendingNotes::default();

        pending_notes.reserve([nullifier]);
        pending_notes.release([nullifier]);

        assert!(pending_notes.nullifiers().is_empty());
    }

    // TODO(RFC-480): a wallet follows the commitment MMRs from genesis.
    /*
    fn ledger_config() -> lb_ledger::Config {
        use std::{
            num::{NonZero, NonZeroU64},
            sync::Arc,
        };

        use lb_core::sdp::{MinStake, ServiceParameters, ServiceType};
        use lb_cryptarchia_engine::EpochConfig;
        use lb_ledger::{
            config::{
                BlendPoWConfig, ConsensusExcludedKeys, ModulusShift, PoWConfig, RewardPoWConfig,
            },
            mantle::sdp::{ServiceRewardsParameters, rewards},
        };
        use lb_utils::math::{NonNegativeRatio, PositiveF64};

        let epoch_config = EpochConfig {
            epoch_stake_distribution_stabilization: NonZero::new(1).unwrap(),
            epoch_period_nonce_buffer: NonZero::new(1).unwrap(),
            epoch_period_nonce_stabilization: NonZero::new(1).unwrap(),
        };
        let consensus_config = lb_cryptarchia_engine::Config::new(
            NonZero::new(1).unwrap(),
            NonNegativeRatio::new(1, 10.try_into().unwrap()),
            1f64.try_into().expect("1 > 0"),
            NonZero::new(12).unwrap(),
        );
        let epoch_length = epoch_config.epoch_length(consensus_config.base_period_length());

        lb_ledger::Config {
            epoch_config,
            consensus_config,
            sdp_config: lb_ledger::mantle::sdp::Config {
                service_params: Arc::new(
                    [(
                        ServiceType::BlendNetwork,
                        ServiceParameters {
                            inactivity_period: 20.try_into().unwrap(),
                            epoch: 0.into(),
                        },
                    )]
                    .into(),
                ),
                service_rewards_params: ServiceRewardsParameters {
                    blend: rewards::blend::RewardsParameters {
                        rounds_per_epoch: epoch_length.try_into().unwrap(),
                        message_frequency_per_round: PositiveF64::try_from(1.0).unwrap(),
                        num_blend_layers: NonZeroU64::new(3).unwrap(),
                        minimum_network_size: NonZeroU64::new(1).unwrap(),
                        data_replication_factor: 0,
                        activity_threshold_sensitivity: 1,
                    },
                },
                min_stake: MinStake {
                    threshold: 1,
                    timestamp: 0,
                },
            },
            consensus_excluded_keys: ConsensusExcludedKeys::empty(),
            pow_config: PoWConfig {
                blend: BlendPoWConfig {
                    base_difficulty: ModulusShift::new::<19>(),
                    damping_den_offset: 0,
                    damping_num: 1.try_into().unwrap(),
                    max_step: 1.try_into().unwrap(),
                    target_transactions_per_block: 1.try_into().unwrap(),
                },
                reward: RewardPoWConfig {
                    reward_pool_genesis: 1_000_000_000,
                    epoch_reward_genesis: 1_000_000,
                    minimum_difficulty: ModulusShift::new::<26>(),
                    ema_smoothing_factor: 9,
                    ema_smoothing_precision: NonZeroU64::new(10).unwrap(),
                    target_claims_per_block: 100,
                    rate_num: 0,
                    rate_den: NonZeroU64::MIN,
                    target_claim_per_block: NonZeroU64::MIN,
                    pow_share: 0,
                    share_den: NonZeroU64::MIN,
                    slot_window: NonZeroU64::new(100).unwrap(),
                },
            },
        }
    }
    */

    /// Regression test for the wallet-service crash seen the first time a
    /// bootstrapping node switches to Online:
    ///
    /// During bootstrap the engine never advances the LIB, so no `LibUpdate`
    /// is broadcast and the wallet's `state.lib` stays at its starting point.
    /// `switch_to_online()` then jumps the chain LIB to the tip's k-th
    /// ancestor without broadcasting a `LibUpdate`. The first block processed
    /// afterwards broadcasts a `LibUpdate` whose `new_lib` can be a block the
    /// wallet has not applied yet (its frontier lags during IBD).
    ///
    /// The service backfills up to `new_lib` before advancing (see
    /// `handle_lib_update`). This test covers the `advance_lib` safety net for
    /// the case the block is still missing: it must skip the advance and keep
    /// the current LIB rather than panicking with
    /// "`WalletState` at LIB must exist: `UnknownBlock(...)`".
    #[test]
    fn lib_update_for_unapplied_block_skips_advance() {
        use std::sync::Arc;

        use lb_services_utils::overwatch::RecoveryData;

        let settings = WalletServiceSettings {
            known_keys: HashMap::new(),
            voucher_master_key_id: "voucher-master".into(),
            recovery_data: RecoveryData::default(),
            pending_note_expiry_blocks: 10,
        };

        // Fresh wallet: initialized from the genesis note sets.
        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // let recovery = RecoveryState {
        //     next_new_voucher_index: 0,
        //     vouchers: Vouchers::default(),
        //     lib_wallet_state: None,
        //     pending_claims: PendingClaims::default(),
        // };
        let recovery = RecoveryState {
            lib_wallet_state: None,
            known_notes: HashMap::new(),
            next_reward_key_index: 0,
        };

        let genesis = HeaderId::from([0; 32]);
        // TODO(RFC-480): a wallet follows the commitment MMRs from genesis.
        // let ledger = LedgerState::from_utxos([], &ledger_config());
        let (sender, _receiver) = tokio::sync::watch::channel(None);
        let updater = StateUpdater::new(Arc::new(sender));

        // TODO(RFC-480): a wallet follows the commitment MMRs from genesis.
        // let mut state = ServiceState::new(recovery, &settings, genesis, &ledger,
        // &updater, 5);
        let mut state =
            ServiceState::from_genesis(recovery, &settings, genesis, &[], &[], [], &updater)
                .unwrap();

        // The first post-online LibUpdate delivers a new_lib the wallet never
        // applied (the silent bootstrap->online LIB jump outran the wallet).
        let unseen_lib = HeaderId::from([42; 32]);
        // TODO(RFC-480): LEADER_CLAIM and the vouchers are removed.
        // state.advance_lib(
        //     unseen_lib,
        //     Vec::<HeaderId>::new(),
        //     1,
        //     Vec::<VoucherNullifier>::new(),
        // );
        state.advance_lib(unseen_lib, Vec::<HeaderId>::new(), 1);

        // No panic: the advance is skipped and the LIB stays at a block whose
        // WalletState exists.
        assert_eq!(state.lib(), genesis);
    }
}
