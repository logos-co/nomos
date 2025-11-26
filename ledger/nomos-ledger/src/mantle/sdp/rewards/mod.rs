pub mod blend;
pub mod da;

use std::{collections::HashMap, num::NonZeroU64};

use groth16::Fr;
use nomos_core::{
    block::BlockNumber,
    sdp::{ActivityMetadata, ProviderId, ServiceParameters, SessionNumber},
};
use thiserror::Error;

use super::SessionState;

pub type RewardAmount = u64;

/// Generic trait for service-specific reward calculation.
///
/// Each service can implement its own rewards logic by implementing this trait.
/// The rewards object is updated with active messages and session transitions,
/// and can calculate expected rewards for each provider based on the service's
/// internal logic.
pub trait Rewards: Clone + PartialEq + Send + Sync + std::fmt::Debug {
    /// Update rewards state when an active message is received.
    ///
    /// Called when a provider submits an active message with metadata
    /// (e.g., activity proofs containing opinions about other providers).
    fn update_active(
        &self,
        declaration_id: ProviderId,
        metadata: &ActivityMetadata,
        block_number: BlockNumber,
    ) -> Result<Self, Error>;

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
    ///
    /// # Arguments
    /// * `last_active` - The state of the session that just ended.
    /// * `next_active_session_epoch_nonce` - The nonce of the epoch state
    ///   corresponding to the 1st block of the session `last_active + 1`.
    fn update_session(
        &self,
        last_active: &SessionState,
        next_active_session_epoch_nonce: &Fr,
        config: &ServiceParameters,
    ) -> (Self, HashMap<ProviderId, RewardAmount>);
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("Rewards state is not initialized yet with a real session")]
    Uninitialized,
    #[error("Invalid session: expected {expected}, got {got}")]
    InvalidSession {
        expected: SessionNumber,
        got: SessionNumber,
    },
    #[error("Invalid opinion length: expected {expected}, got {got}")]
    InvalidOpinionLength { expected: usize, got: usize },
    #[error("Duplicate active message for session {session}, provider {provider_id:?}")]
    DuplicateActiveMessage {
        session: SessionNumber,
        provider_id: Box<ProviderId>,
    },
    #[error("Invalid proof type")]
    InvalidProofType,
    #[error("Invalid proof")]
    InvalidProof,
    #[error(
        "The number of declarations ({num_declarations}) is less than the minimum network size ({minimum_network_size})"
    )]
    MinimumNetworkSizeNotSatisfied {
        num_declarations: u64,
        minimum_network_size: NonZeroU64,
    },
}

#[cfg(test)]
mod tests {
    use nomos_core::sdp::{Declaration, DeclarationId, ServiceType};
    use num_bigint::BigUint;
    use zksign::PublicKey;

    use super::*;

    pub fn create_test_session_state(
        provider_ids: &[ProviderId],
        service_type: ServiceType,
        session_n: SessionNumber,
    ) -> SessionState {
        let mut declarations = rpds::RedBlackTreeMapSync::new_sync();
        for (i, provider_id) in provider_ids.iter().enumerate() {
            let declaration = Declaration {
                service_type,
                provider_id: *provider_id,
                locked_note_id: Fr::from(i as u64).into(),
                locators: vec![],
                zk_id: PublicKey::new(BigUint::from(i as u64).into()),
                created: 0,
                active: 0,
                withdrawn: None,
                nonce: 0,
            };
            declarations = declarations.insert(DeclarationId([i as u8; 32]), declaration);
        }
        SessionState {
            declarations,
            session_n,
        }
    }

    pub fn create_provider_id(byte: u8) -> ProviderId {
        use ed25519_dalek::SigningKey;
        let key_bytes = [byte; 32];
        // Ensure the key is valid by using SigningKey
        let signing_key = SigningKey::from_bytes(&key_bytes);
        ProviderId(signing_key.verifying_key())
    }

    pub fn create_service_parameters() -> ServiceParameters {
        ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 100,
            timestamp: 0,
            session_duration: 10,
        }
    }
}
