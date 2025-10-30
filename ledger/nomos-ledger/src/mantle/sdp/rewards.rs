use std::collections::HashMap;

use nomos_core::{
    block::BlockNumber,
    sdp::{ActivityMetadata, DeclarationId, ProviderId, ServiceParameters},
};

use super::SessionState;

pub type RewardAmount = u64;

/// Generic trait for service-specific reward calculation.
///
/// Each service can implement its own rewards logic by implementing this trait.
/// The rewards object is updated with active messages and session transitions,
/// and can calculate expected rewards for each provider based on the service's
/// internal logic.
pub trait Rewards: Clone + PartialEq + Eq + Send + Sync + std::fmt::Debug {
    /// Update rewards state when an active message is received.
    ///
    /// Called when a provider submits an active message with metadata
    /// (e.g., activity proofs containing opinions about other providers).
    fn update_active(
        &mut self,
        declaration_id: DeclarationId,
        metadata: &ActivityMetadata,
        block_number: BlockNumber,
    );

    /// Update rewards state when sessions transition and calculate rewards to
    /// distribute.
    ///
    /// Called during session boundaries when active, `past_session`, and
    /// forming sessions are updated. Returns a map of `ProviderId` to
    /// reward amounts for providers eligible for rewards in this session
    /// transition.
    ///
    /// The internal calculation logic is opaque to the SDP ledger and
    /// determined by the service-specific implementation.
    fn update_session(
        &mut self,
        active: &SessionState,
        past_session: &SessionState,
        forming: &SessionState,
        block_number: BlockNumber,
        config: &ServiceParameters,
    ) -> HashMap<ProviderId, RewardAmount>;
}

/// No-op rewards implementation that doesn't track or distribute any rewards.
///
/// This is used as a placeholder for services that don't implement rewards yet.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoopRewards;

impl Rewards for NoopRewards {
    fn update_active(
        &mut self,
        _declaration_id: DeclarationId,
        _metadata: &ActivityMetadata,
        _block_number: BlockNumber,
    ) {
        // No-op: this implementation doesn't track rewards
    }

    fn update_session(
        &mut self,
        _active: &SessionState,
        _past_session: &SessionState,
        _forming: &SessionState,
        _block_number: BlockNumber,
        _config: &ServiceParameters,
    ) -> HashMap<ProviderId, RewardAmount> {
        // No rewards to distribute
        HashMap::new()
    }
}
