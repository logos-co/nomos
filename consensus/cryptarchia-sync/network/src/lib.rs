pub mod behaviour;

pub mod membership;
mod sync_incoming;
mod sync_outgoing;
mod sync_utils;


// TODO: find a better place(that is not behind libp2p feature flag)
#[derive(Debug, Clone)]
pub enum SyncRequestKind {
    ForwardChain(u64),
    BackwardChain([u8; 32]),
    Tip
}