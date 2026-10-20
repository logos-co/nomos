use std::num::NonZeroU64;

// TODO(RFC-480): service notes live in the SDP note set and are consumed by
// SDP_WITHDRAW; `service_notes::ServiceNotes` left the `sdp` imports.
// TODO(RFC-480): no Operation reads legacy UTXOs since M9. `Utxos` left the
// `ledger` imports.
use lb_core::{
    block::BlockNumber,
    crypto::{Hash, ZkHash},
    mantle::{
        channel::Channels,
        ledger::Declarations,
        note_set::NoteSets,
        ops::{
            channel::{ChannelId, ChannelKeyIndex},
            // TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are
            // retired; the leader reward is a note in the block.
            // leader_claim::{RewardsRoot, VoucherNullifier},
            pow::{PowNullifier, PowReward, PowTarget},
            sdp::SdpError,
        },
        programs::Programs,
        transactions::{OperationVerificationHelper, VerificationError},
    },
    sdp::{DeclarationId, MinStake, ServiceType},
};
use lb_cryptarchia_engine::{Epoch, Slot};
use lb_key_management_system_keys::keys::UnverifiedEd25519PublicKey;
// TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are retired; the leader
// reward is a note in the block. `HashTrieSetSync` held the voucher nullifiers.
use rpds::HashTrieMapSync;

use crate::mantle::LedgerState;

pub struct MantleOperationVerificationHelper<'a> {
    ledger_state: &'a LedgerState,
    cryptarchia_ledger: &'a crate::CryptarchiaLedger,
    block_number: BlockNumber,
    config: &'a crate::Config,
}

impl<'a> MantleOperationVerificationHelper<'a> {
    #[must_use]
    pub const fn new(
        ledger_state: &'a LedgerState,
        cryptarchia_ledger: &'a crate::CryptarchiaLedger,
        block_number: BlockNumber,
        config: &'a crate::Config,
    ) -> Self {
        Self {
            ledger_state,
            cryptarchia_ledger,
            block_number,
            config,
        }
    }
}

impl OperationVerificationHelper for MantleOperationVerificationHelper<'_> {
    fn get_channels(&self) -> &Channels {
        self.ledger_state.channels()
    }

    // TODO(RFC-480): service notes live in the SDP note set and are consumed by
    // SDP_WITHDRAW.
    // fn get_service_notes(&self) -> &ServiceNotes {
    //     self.ledger_state.service_notes()
    // }

    // TODO(RFC-480): no Operation reads legacy UTXOs since M9.
    // fn get_utxos(&self) -> &Utxos {
    //     &self.cryptarchia_ledger.utxos
    // }

    fn get_note_sets(&self) -> &NoteSets {
        &self.cryptarchia_ledger.note_sets
    }

    fn get_programs(&self) -> &Programs {
        self.ledger_state.programs()
    }

    fn get_block_number(&self) -> BlockNumber {
        self.block_number
    }

    fn get_declarations_by_service(
        &self,
        service: ServiceType,
    ) -> Result<&Declarations, VerificationError> {
        self.ledger_state
            .sdp_ledger()
            .get_declarations_by_service(service)
            .ok_or(VerificationError::SDPVerificationError(
                SdpError::ServiceNotFound(service),
            ))
    }

    fn get_declarations_by_id(
        &self,
        id: &DeclarationId,
    ) -> Result<&Declarations, VerificationError> {
        self.ledger_state
            .sdp_ledger()
            .get_declarations_by_id(id)
            .ok_or(VerificationError::SDPVerificationError(
                SdpError::DeclarationNotFound(*id),
            ))
    }

    fn get_min_stake(&self) -> &MinStake {
        &self.config.sdp_config.min_stake
    }

    fn get_epoch(&self) -> Epoch {
        // TODO(RFC-480): the header carries no voucher; LeaderState goes with
        // LEADER_CLAIM.
        // self.ledger_state.leaders.epoch()
        self.cryptarchia_ledger.epoch_state().epoch()
    }

    fn get_block_slot(&self) -> Slot {
        self.cryptarchia_ledger.slot
    }

    // TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are retired; the
    // leader reward is a note in the block.
    // fn get_nullifiers(&self) -> &HashTrieSetSync<VoucherNullifier> {
    //     self.ledger_state.leaders.nullifiers()
    // }
    //
    // fn get_claimable_vouchers_root(&self) -> &RewardsRoot {
    //     self.ledger_state.vouchers_snapshot_root()
    // }

    // TODO(RFC-480): transfer_threshold is removed, holders
    // withdraw their own channel notes.
    // fn get_channel_transfer_threshold(
    //     &self,
    //     channel_id: &ChannelId,
    // ) -> Result<ChannelKeyIndex, VerificationError> {
    //     self.ledger_state
    //         .channels()
    //         .channel_state(channel_id)
    //         .ok_or(VerificationError::ChannelNotFound {
    //             channel_id: *channel_id,
    //         })
    //         .map(|channel_state| channel_state.transfer_threshold)
    // }

    fn get_key_from_channel_at_index(
        &self,
        channel_id: &ChannelId,
        key_index: &ChannelKeyIndex,
    ) -> Result<UnverifiedEd25519PublicKey, VerificationError> {
        self.ledger_state
            .channels()
            .channel_state(channel_id)
            .ok_or(VerificationError::ChannelNotFound {
                channel_id: *channel_id,
            })?
            .accredited_keys
            .get(*key_index as usize)
            .ok_or(VerificationError::KeyNotFound {
                channel_id: *channel_id,
                key_index: *key_index,
            })
            .cloned()
    }

    fn get_pow_reward_difficulty(&self) -> PowTarget {
        self.ledger_state.pow.reward_difficulty()
    }

    fn get_pow_nullifiers(&self) -> &HashTrieMapSync<PowNullifier, Slot> {
        self.ledger_state.pow.nullifiers()
    }

    fn get_epoch_pow_reward(&self) -> PowReward {
        self.ledger_state.pow.epoch_reward()
    }

    fn get_pow_reward_pool(&self) -> PowReward {
        self.ledger_state.pow.reward_pool()
    }

    fn get_current_epoch_nonce(&self) -> ZkHash {
        self.cryptarchia_ledger.epoch_state.nonce
    }

    fn get_previous_epoch_nonce(&self) -> ZkHash {
        self.cryptarchia_ledger.previous_epoch_nonce
    }

    fn get_blocks_slot(&self) -> HashTrieMapSync<Hash, Slot> {
        self.ledger_state.pow.block_slots().clone()
    }

    fn get_pow_slot_window(&self) -> NonZeroU64 {
        self.config.pow_config.reward.slot_window
    }
}
