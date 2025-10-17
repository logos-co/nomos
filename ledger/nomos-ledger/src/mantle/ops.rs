use std::{
    collections::{HashMap, HashSet},
    mem,
    sync::Arc,
};

use ed25519::signature::Verifier as _;
use nomos_core::{
    block::BlockNumber,
    mantle::{
        NoteId, TxHash,
        ops::{
            channel::{
                ChannelId, Ed25519PublicKey as PublicKey, MsgId, blob::BlobOp,
                inscribe::InscriptionOp, set_keys::SetKeysOp,
            },
            leader_claim::LeaderClaimOp,
            sdp::{SDPActiveOp, SDPDeclareOp, SDPWithdrawOp},
        },
    },
    proofs::zksig::{DummyZkSignature, ZkSignatureProof as _},
    sdp::ServiceType,
};

use super::{LedgerState, channel, locked_notes, sdp};
use crate::{Balance, Config, UtxoTree};

#[derive(Clone)]
enum NoteDelta {
    Lock(NoteId, ServiceType),
    Unlock(NoteId, ServiceType),
}

/// Minimal validation context for multi-operation transaction validation
pub(super) struct ValidationContext {
    /// Only the channels that operations will affect
    channel_states: HashMap<ChannelId, channel::ChannelState>,
    /// Working copy of SDP ledger with updates applied during validation
    working_sdp: sdp::SdpLedger,
    /// In-transaction note lock state overrides
    note_state: HashMap<(NoteId, ServiceType), bool>,
    /// Ordered list of lock/unlock operations for commit
    note_deltas: Vec<NoteDelta>,
}

struct SdpOperationContext<'a> {
    original_ledger: &'a LedgerState,
    current_block_number: BlockNumber,
    config: &'a Config,
    utxo_tree: &'a UtxoTree,
    tx_hash: &'a TxHash,
}

impl ValidationContext {
    /// Create context with only the data structures that operations will affect
    pub fn extract_for_ops<'a, I>(ledger: &LedgerState, ops: I) -> Self
    where
        I: IntoIterator<Item = (&'a super::Op, Option<&'a super::OpProof>)>,
    {
        let ops_vec: Vec<_> = ops.into_iter().collect();

        let affected_channels: HashSet<ChannelId> = ops_vec
            .iter()
            .filter_map(|(op, _)| match op {
                super::Op::ChannelBlob(op) => Some(op.channel),
                super::Op::ChannelInscribe(op) => Some(op.channel_id),
                super::Op::ChannelSetKeys(op) => Some(op.channel),
                _ => None,
            })
            .collect();

        Self {
            channel_states: affected_channels
                .iter()
                .filter_map(|&id| {
                    ledger
                        .channels
                        .channels
                        .get(&id)
                        .map(|state| (id, Arc::clone(&state.keys), state.tip))
                })
                .map(|(id, keys, tip)| (id, channel::ChannelState { tip, keys }))
                .collect(),
            working_sdp: ledger.sdp.clone(),
            note_state: HashMap::new(),
            note_deltas: Vec::new(),
        }
    }

    /// Get current channel tip (considering updates within transaction)
    pub fn get_channel_tip(
        &self,
        original_channels: &channel::Channels,
        channel_id: ChannelId,
    ) -> MsgId {
        self.channel_states
            .get(&channel_id)
            .map(|state| state.tip)
            .or_else(|| {
                original_channels
                    .channels
                    .get(&channel_id)
                    .map(|state| state.tip)
            })
            .unwrap_or(MsgId::root())
    }

    /// Update channel tip for next operations in transaction
    pub fn update_channel_tip(&mut self, channel_id: ChannelId, new_tip: MsgId, signer: PublicKey) {
        let keys = self.channel_states.get(&channel_id).map_or_else(
            || Arc::from(vec![signer].into_boxed_slice()),
            |state| Arc::clone(&state.keys),
        );
        self.channel_states
            .insert(channel_id, channel::ChannelState { tip: new_tip, keys });
    }

    fn handle_channel_blob(
        &mut self,
        original_ledger: &LedgerState,
        op: &BlobOp,
    ) -> Result<(), super::Error> {
        let current_tip = self.get_channel_tip(&original_ledger.channels, op.channel);
        if op.parent != current_tip {
            return Err(super::Error::Channel(channel::Error::InvalidParent {
                channel_id: op.channel,
                parent: op.parent.into(),
                actual: current_tip.into(),
            }));
        }

        if self
            .channel_states
            .get(&op.channel)
            .or_else(|| original_ledger.channels.channels.get(&op.channel))
            .is_some_and(|channel| !channel.keys.contains(&op.signer))
        {
            return Err(super::Error::Channel(channel::Error::UnauthorizedSigner {
                channel_id: op.channel,
                signer: format!("{:?}", op.signer),
            }));
        }

        self.update_channel_tip(op.channel, op.id(), op.signer);
        Ok(())
    }

    fn handle_channel_inscribe(
        &mut self,
        original_ledger: &LedgerState,
        op: &InscriptionOp,
    ) -> Result<(), super::Error> {
        let current_tip = self.get_channel_tip(&original_ledger.channels, op.channel_id);
        if op.parent != current_tip {
            return Err(super::Error::Channel(channel::Error::InvalidParent {
                channel_id: op.channel_id,
                parent: op.parent.into(),
                actual: current_tip.into(),
            }));
        }

        if self
            .channel_states
            .get(&op.channel_id)
            .or_else(|| original_ledger.channels.channels.get(&op.channel_id))
            .is_some_and(|channel| !channel.keys.contains(&op.signer))
        {
            return Err(super::Error::Channel(channel::Error::UnauthorizedSigner {
                channel_id: op.channel_id,
                signer: format!("{:?}", op.signer),
            }));
        }

        self.update_channel_tip(op.channel_id, op.id(), op.signer);
        Ok(())
    }

    fn handle_channel_set_keys(
        &mut self,
        original_ledger: &LedgerState,
        tx_hash: &TxHash,
        op: &SetKeysOp,
        sig: &ed25519::Signature,
    ) -> Result<(), super::Error> {
        if self
            .channel_states
            .get(&op.channel)
            .or_else(|| original_ledger.channels.channels.get(&op.channel))
            .is_some_and(|channel| {
                channel.keys[0]
                    .verify(tx_hash.as_signing_bytes().as_ref(), sig)
                    .is_err()
            })
        {
            return Err(super::Error::Channel(channel::Error::InvalidSignature));
        }

        if op.keys.is_empty() {
            return Err(super::Error::Channel(channel::Error::EmptyKeys {
                channel_id: op.channel,
            }));
        }

        let current_tip = self.get_channel_tip(&original_ledger.channels, op.channel);
        self.channel_states.insert(
            op.channel,
            channel::ChannelState {
                tip: current_tip,
                keys: op.keys.clone().into(),
            },
        );

        Ok(())
    }

    fn handle_sdp_declare(
        &mut self,
        ctx: &SdpOperationContext<'_>,
        op: &SDPDeclareOp,
        zk_sig: &DummyZkSignature,
        ed25519_sig: &ed25519::Signature,
    ) -> Result<(), super::Error> {
        let Some((utxo, _)) = ctx.utxo_tree.utxos().get(&op.locked_note_id) else {
            return Err(super::Error::NoteNotFound(op.locked_note_id));
        };
        if !zk_sig.verify(&super::zksig::ZkSignaturePublic {
            pks: vec![utxo.note.pk.into(), op.zk_id.0],
            msg_hash: ctx.tx_hash.0,
        }) {
            return Err(super::Error::InvalidSignature);
        }
        op.provider_id
            .0
            .verify(ctx.tx_hash.as_signing_bytes().as_ref(), ed25519_sig)
            .map_err(|_| super::Error::InvalidSignature)?;

        if self.is_note_locked(
            &ctx.original_ledger.locked_notes,
            &op.locked_note_id,
            op.service_type,
        ) {
            return Err(super::Error::LockedNotes(
                locked_notes::Error::NoteAlreadyUsedForService {
                    note_id: op.locked_note_id,
                    service_type: op.service_type,
                },
            ));
        }

        if utxo.note.value < ctx.config.min_stake.threshold {
            return Err(super::Error::LockedNotes(
                locked_notes::Error::NoteInsufficientValue {
                    note_id: op.locked_note_id,
                    value: utxo.note.value,
                },
            ));
        }

        let current_sdp = mem::take(&mut self.working_sdp);
        self.working_sdp = current_sdp.apply_declare_msg(ctx.current_block_number, op)?;

        self.lock_note(
            &ctx.original_ledger.locked_notes,
            op.locked_note_id,
            op.service_type,
        );

        Ok(())
    }

    fn handle_sdp_active(
        &mut self,
        ctx: &SdpOperationContext<'_>,
        op: &SDPActiveOp,
        sig: &DummyZkSignature,
    ) -> Result<(), super::Error> {
        let declaration = self
            .working_sdp
            .declarations
            .get(&op.declaration_id)
            .or_else(|| ctx.original_ledger.sdp.declarations.get(&op.declaration_id))
            .cloned()
            .ok_or(super::Error::Sdp(
                sdp::SdpLedgerError::SdpDeclarationNotFound(op.declaration_id),
            ))?;

        let service_type = declaration.service_type;
        let Some((utxo, _)) = ctx.utxo_tree.utxos().get(&declaration.locked_note_id) else {
            return Err(super::Error::NoteNotFound(declaration.locked_note_id));
        };
        if !sig.verify(&super::zksig::ZkSignaturePublic {
            pks: vec![utxo.note.pk.into(), declaration.zk_id.0],
            msg_hash: ctx.tx_hash.0,
        }) {
            return Err(super::Error::InvalidSignature);
        }

        let current_sdp = mem::take(&mut self.working_sdp);
        self.working_sdp = current_sdp.apply_active_msg(
            ctx.current_block_number,
            ctx.config
                .service_params
                .get(&service_type)
                .ok_or(super::Error::ServiceParamsNotFound(service_type))?,
            op,
        )?;

        Ok(())
    }

    fn handle_sdp_withdraw(
        &mut self,
        ctx: &SdpOperationContext<'_>,
        op: &SDPWithdrawOp,
        sig: &DummyZkSignature,
    ) -> Result<(), super::Error> {
        let declaration = self
            .working_sdp
            .declarations
            .get(&op.declaration_id)
            .or_else(|| ctx.original_ledger.sdp.declarations.get(&op.declaration_id))
            .cloned()
            .ok_or(super::Error::Sdp(
                sdp::SdpLedgerError::SdpDeclarationNotFound(op.declaration_id),
            ))?;

        let service_type = declaration.service_type;
        let Some((utxo, _)) = ctx.utxo_tree.utxos().get(&declaration.locked_note_id) else {
            return Err(super::Error::NoteNotFound(declaration.locked_note_id));
        };
        if !sig.verify(&super::zksig::ZkSignaturePublic {
            pks: vec![utxo.note.pk.into(), declaration.zk_id.0],
            msg_hash: ctx.tx_hash.0,
        }) {
            return Err(super::Error::InvalidSignature);
        }

        if !self.is_note_locked(
            &ctx.original_ledger.locked_notes,
            &declaration.locked_note_id,
            declaration.service_type,
        ) {
            return Err(super::Error::LockedNotes(
                locked_notes::Error::NoteNotLockedForService {
                    note_id: declaration.locked_note_id,
                    service_type: declaration.service_type,
                },
            ));
        }

        let current_sdp = mem::take(&mut self.working_sdp);
        self.working_sdp = current_sdp.apply_withdrawn_msg(
            ctx.current_block_number,
            ctx.config
                .service_params
                .get(&service_type)
                .ok_or(super::Error::ServiceParamsNotFound(service_type))?,
            op,
        )?;

        self.unlock_note(
            &ctx.original_ledger.locked_notes,
            declaration.locked_note_id,
            declaration.service_type,
        );

        Ok(())
    }

    fn handle_leader_claim(
        original_ledger: &LedgerState,
        op: &LeaderClaimOp,
    ) -> Result<Balance, super::Error> {
        Ok(original_ledger.leaders.validate_claim(op)?)
    }

    /// Check if note is locked (considering lock/unlock operations in
    /// transaction)
    pub fn is_note_locked(
        &self,
        original_locked_notes: &locked_notes::LockedNotes,
        note_id: &NoteId,
        service_type: ServiceType,
    ) -> bool {
        self.note_state
            .get(&(*note_id, service_type))
            .copied()
            .unwrap_or_else(|| original_locked_notes.is_locked_for_service(note_id, service_type))
    }

    /// Track that a note becomes locked in this transaction
    pub fn lock_note(
        &mut self,
        original_locked_notes: &locked_notes::LockedNotes,
        note_id: NoteId,
        service_type: ServiceType,
    ) {
        let key = (note_id, service_type);
        let entry = self
            .note_state
            .entry(key)
            .or_insert_with(|| original_locked_notes.is_locked_for_service(&note_id, service_type));
        *entry = true;
        self.note_deltas
            .push(NoteDelta::Lock(note_id, service_type));
    }

    /// Track that a note becomes unlocked in this transaction
    pub fn unlock_note(
        &mut self,
        original_locked_notes: &locked_notes::LockedNotes,
        note_id: NoteId,
        service_type: ServiceType,
    ) {
        let key = (note_id, service_type);
        let entry = self
            .note_state
            .entry(key)
            .or_insert_with(|| original_locked_notes.is_locked_for_service(&note_id, service_type));
        *entry = false;
        self.note_deltas
            .push(NoteDelta::Unlock(note_id, service_type));
    }

    /// Process operations with context tracking (for both validation and
    /// application)
    pub fn process_operations<'a>(
        &mut self,
        original_ledger: &LedgerState,
        current_block_number: BlockNumber,
        config: &Config,
        utxo_tree: &UtxoTree,
        tx_hash: TxHash,
        ops: impl Iterator<Item = (&'a super::Op, Option<&'a super::OpProof>)> + 'a,
    ) -> Result<Balance, super::Error> {
        let mut balance = 0;
        let sdp_ctx = SdpOperationContext {
            original_ledger,
            current_block_number,
            config,
            utxo_tree,
            tx_hash: &tx_hash,
        };

        for (op, proof) in ops {
            let delta = match (op, proof) {
                (super::Op::ChannelBlob(op), Some(super::OpProof::Ed25519Sig(_))) => {
                    self.handle_channel_blob(original_ledger, op)?;
                    0
                }

                (super::Op::ChannelInscribe(op), Some(super::OpProof::Ed25519Sig(_))) => {
                    self.handle_channel_inscribe(original_ledger, op)?;
                    0
                }

                (super::Op::ChannelSetKeys(op), Some(super::OpProof::Ed25519Sig(sig))) => {
                    self.handle_channel_set_keys(original_ledger, &tx_hash, op, sig)?;
                    0
                }

                (
                    super::Op::SDPDeclare(op),
                    Some(super::OpProof::ZkAndEd25519Sigs {
                        zk_sig,
                        ed25519_sig,
                    }),
                ) => {
                    self.handle_sdp_declare(&sdp_ctx, op, zk_sig, ed25519_sig)?;
                    0
                }

                (super::Op::SDPActive(op), Some(super::OpProof::ZkSig(sig))) => {
                    self.handle_sdp_active(&sdp_ctx, op, sig)?;
                    0
                }

                (super::Op::SDPWithdraw(op), Some(super::OpProof::ZkSig(sig))) => {
                    self.handle_sdp_withdraw(&sdp_ctx, op, sig)?;
                    0
                }

                (super::Op::LeaderClaim(op), None) => {
                    Self::handle_leader_claim(original_ledger, op)?
                }

                _ => {
                    return Err(super::Error::UnsupportedOp);
                }
            };

            balance += delta;
        }

        Ok(balance)
    }

    /// Commit all tracked changes to the real ledger state (for application)
    pub fn commit_to_ledger(self, ledger: &mut LedgerState) {
        let Self {
            channel_states,
            working_sdp,
            note_deltas,
            ..
        } = self;

        for (channel_id, new_state) in channel_states {
            ledger.channels.channels = ledger.channels.channels.insert(channel_id, new_state);
        }

        ledger.sdp = working_sdp;

        for delta in note_deltas {
            match delta {
                NoteDelta::Lock(note_id, service_type) => {
                    ledger.locked_notes = ledger
                        .locked_notes
                        .clone()
                        .lock_unchecked(service_type, &note_id);
                }
                NoteDelta::Unlock(note_id, service_type) => {
                    ledger.locked_notes = ledger
                        .locked_notes
                        .clone()
                        .unlock_unchecked(service_type, &note_id);
                }
            }
        }
    }
}
