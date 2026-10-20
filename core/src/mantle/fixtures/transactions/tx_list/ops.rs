use lb_binary_codec::canonical::codec_fixtures;

use crate::mantle::{
    Op,
    fixtures::ops::op_values::{
        ALL_OPS_COLUMN_HEX, CHANNEL_CONFIG, CHANNEL_WITHDRAW, CLAIM_POW_REWARD, DEPOSIT,
        EMPTY_COLUMN_HEX, INSCRIPTION,
        // TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are retired;
        // the leader reward is a note in the block.
        // LEADER_CLAIM,
        SDP_ACTIVE, SDP_DECLARE, SDP_WITHDRAW, TRANSFER, TRANSFER_AND_INSCRIPTION_COLUMN_HEX,
        TRANSFER_COLUMN_HEX,
    },
    transactions::Ops,
};

codec_fixtures!(
    Ops,
    Self::empty() => EMPTY_COLUMN_HEX,
    Self::from([Op::Transfer(TRANSFER.clone())]) => TRANSFER_COLUMN_HEX,
    Self::from([
        Op::Transfer(TRANSFER.clone()),
        Op::ChannelInscribe(INSCRIPTION.clone()),
    ]) => TRANSFER_AND_INSCRIPTION_COLUMN_HEX,
    Self::from([
        Op::Transfer(TRANSFER.clone()),
        Op::ChannelConfig(CHANNEL_CONFIG.clone()),
        Op::ChannelInscribe(INSCRIPTION.clone()),
        Op::ChannelDeposit(DEPOSIT.clone()),
        Op::ChannelWithdraw(CHANNEL_WITHDRAW.clone()),
        // TODO(RFC-480): CHANNEL_TRANSFER is removed, channel notes move by
        // the steps of CHANNEL_INSCRIBE.
        // Op::ChannelTransfer(CHANNEL_TRANSFER.clone()),
        Op::SDPDeclare(SDP_DECLARE.clone()),
        Op::SDPWithdraw(SDP_WITHDRAW.clone()),
        Op::SDPActive(SDP_ACTIVE.clone()),
        // TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are retired;
        // the leader reward is a note in the block.
        // Op::LeaderClaim(LEADER_CLAIM.clone()),
        Op::ClaimPowReward(CLAIM_POW_REWARD.clone()),
    ]) => ALL_OPS_COLUMN_HEX
);
