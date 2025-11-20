use nomos_core::{
    block::Block,
    mantle::{
        AuthenticatedMantleTx as _, SignedMantleTx,
        ops::{Op, channel::MsgId},
    },
};

/// Scans a block and invokes the matcher for every operation until it returns
/// `Some(...)`. Returns `None` when no matching operation is found.
pub fn find_channel_op<F>(block: &Block<SignedMantleTx>, matcher: &mut F) -> Option<MsgId>
where
    F: FnMut(&Op) -> Option<MsgId>,
{
    for tx in block.transactions() {
        for op in &tx.mantle_tx().ops {
            if let Some(msg_id) = matcher(op) {
                return Some(msg_id);
            }
        }
    }

    None
}
