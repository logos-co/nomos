use std::convert::Infallible;
// std
use std::error::Error;
use std::hash::Hash;

// crates

// internal
use carnot_engine::overlay::{
    CommitteeMembership, Error as RandomBeaconError, FreezeMembership, RandomBeaconState,
};
use carnot_engine::TimeoutQc;
use nomos_core::block::Block;

pub trait UpdateableCommitteeMembership: CommitteeMembership {
    type Error: Error;

    fn on_new_block_received<Tx: Hash + Clone + Eq, Blob: Clone + Eq + Hash>(
        &self,
        block: &Block<Tx, Blob>,
    ) -> Result<Self, Self::Error>;
    fn on_timeout_qc_received(&self, qc: &TimeoutQc) -> Result<Self, Self::Error>;
}

impl UpdateableCommitteeMembership for FreezeMembership {
    type Error = Infallible;

    fn on_new_block_received<Tx: Hash + Clone + Eq, Blob: Clone + Eq + Hash>(
        &self,
        _block: &Block<Tx, Blob>,
    ) -> Result<Self, Self::Error> {
        Ok(Self)
    }

    fn on_timeout_qc_received(&self, _qc: &TimeoutQc) -> Result<Self, Self::Error> {
        Ok(Self)
    }
}

impl UpdateableCommitteeMembership for RandomBeaconState {
    type Error = RandomBeaconError;

    fn on_new_block_received<Tx: Hash + Clone + Eq, Blob: Clone + Eq + Hash>(
        &self,
        block: &Block<Tx, Blob>,
    ) -> Result<Self, Self::Error> {
        self.check_advance_happy(block.beacon().clone(), block.header().parent_qc.view())
    }

    fn on_timeout_qc_received(&self, qc: &TimeoutQc) -> Result<Self, Self::Error> {
        Ok(Self::generate_sad(qc.view(), self))
    }
}
