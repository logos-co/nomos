pub mod powclaim;
pub mod ttl;

pub use powclaim::PoWClaimPolicy;
pub use ttl::TtlPolicy;

pub struct PolicyTime {
    now: u64,
    slot: Slot,
}
pub enum PolicyEvent<Tx> {
    Add { tx: Tx, now: PolicyTime },
    Remove { tx: Tx },
    Select { now: PolicyTime },
}

pub trait TxEvictionPolicy {
    type Tx;
    type Settings;
    fn new(settings: Self::Settings) -> Self;
    fn process_event(&mut self, event: PolicyEvent<Self::Tx>) -> bool;
}
