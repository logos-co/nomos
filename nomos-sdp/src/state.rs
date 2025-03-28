use std::fmt::Display;

use crate::{BlockNumber, DeclarationId, EventType, ProviderId, ProviderInfo, ServiceParameters};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ActiveState(ProviderInfo);

impl ActiveState {
    const fn to_updated(
        mut self,
        block_number: BlockNumber,
        event_type: EventType,
    ) -> Result<Self, ProviderStateError> {
        match event_type {
            EventType::Reward => {
                self.0.rewarded = Some(block_number);
                Ok(self)
            }
            EventType::Declaration | EventType::Withdrawal => {
                Err(ProviderStateError::InvalidTransition)
            }
        }
    }

    const fn to_withdrawn(
        self,
        block_number: BlockNumber,
        event_type: EventType,
        service_params: &ServiceParameters,
    ) -> Result<WithdrawnState, ProviderStateError> {
        match event_type {
            EventType::Withdrawal => {
                if self.0.created.wrapping_add(service_params.lock_period) >= block_number {
                    return Err(ProviderStateError::InvalidTransition);
                }
                Ok(WithdrawnState(ProviderInfo {
                    provider_id: self.0.provider_id,
                    declaration_id: self.0.declaration_id,
                    created: self.0.created,
                    rewarded: self.0.rewarded,
                    withdrawn: Some(block_number),
                }))
            }
            EventType::Declaration | EventType::Reward => {
                Err(ProviderStateError::InvalidTransition)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct InactiveState(ProviderInfo);

impl InactiveState {
    const fn to_active(
        mut self,
        block_number: BlockNumber,
        event_type: EventType,
    ) -> Result<ActiveState, ProviderStateError> {
        match event_type {
            EventType::Reward => {
                self.0.rewarded = Some(block_number);
                Ok(ActiveState(self.0))
            }
            EventType::Declaration | EventType::Withdrawal => {
                Err(ProviderStateError::InvalidTransition)
            }
        }
    }

    const fn to_withdrawn(
        self,
        block_number: BlockNumber,
        event_type: EventType,
        service_params: &ServiceParameters,
    ) -> Result<WithdrawnState, ProviderStateError> {
        match event_type {
            EventType::Withdrawal => {
                if self.0.created.wrapping_add(service_params.lock_period) >= block_number {
                    return Err(ProviderStateError::InvalidTransition);
                }
                Ok(WithdrawnState(ProviderInfo {
                    provider_id: self.0.provider_id,
                    declaration_id: self.0.declaration_id,
                    created: self.0.created,
                    rewarded: self.0.rewarded,
                    withdrawn: Some(block_number),
                }))
            }
            EventType::Declaration | EventType::Reward => {
                Err(ProviderStateError::InvalidTransition)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct WithdrawnState(ProviderInfo);

impl WithdrawnState {
    fn to_withdrawn_rewarded(
        mut self,
        block_number: BlockNumber,
        event_type: EventType,
    ) -> Result<Self, ProviderStateError> {
        if matches!(event_type, EventType::Reward) {
            // Only allow reward for withdrawal in the same block number.
            if self.0.withdrawn != Some(block_number) {
                return Err(ProviderStateError::WrongBlockNumber);
            }

            self.0.rewarded = Some(block_number);
            Ok(self)
        } else {
            Err(ProviderStateError::InvalidTransition)
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ProviderStateError {
    #[error("Invalid provider state transition")]
    InvalidTransition,
    #[error("ProviderId is used in another declaration")]
    WrongDeclarationId,
    #[error("Block number does not match state number")]
    WrongBlockNumber,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum ProviderState {
    New(ProviderInfo),
    Active(ActiveState),
    Inactive(InactiveState),
    Withdrawn(WithdrawnState),
}

impl Display for ProviderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::New(_) => write!(f, "New"),
            Self::Active(_) => write!(f, "Active"),
            Self::Inactive(_) => write!(f, "Inactive"),
            Self::Withdrawn(_) => write!(f, "Withdrawn"),
        }
    }
}

impl ProviderState {
    #[must_use]
    pub const fn new(
        block_number: BlockNumber,
        provider_id: ProviderId,
        declaration_id: DeclarationId,
    ) -> Self {
        let provider_info = ProviderInfo::new(provider_id, declaration_id, block_number);
        Self::New(provider_info)
    }

    #[must_use]
    pub const fn declaration_id(&self) -> DeclarationId {
        match self {
            Self::New(provider_info) => provider_info.declaration_id,
            Self::Active(active_state) => active_state.0.declaration_id,
            Self::Inactive(inactive_state) => inactive_state.0.declaration_id,
            Self::Withdrawn(withdrawn_state) => withdrawn_state.0.declaration_id,
        }
    }

    #[must_use]
    pub fn from_info(
        block_number: BlockNumber,
        provider_info: ProviderInfo,
        service_params: &ServiceParameters,
    ) -> Self {
        if provider_info.withdrawn.is_some() {
            return WithdrawnState(provider_info).into();
        }

        if let Some(rewarded) = provider_info.rewarded {
            if block_number.wrapping_sub(rewarded) <= service_params.inactivity_period {
                return ActiveState(provider_info).into();
            }
        }

        InactiveState(provider_info).into()
    }

    pub fn to_active(
        self,
        block_number: BlockNumber,
        event_type: EventType,
    ) -> Result<Self, ProviderStateError> {
        match self {
            Self::New(info) => Ok(ActiveState(info).into()),
            Self::Active(active_state) => active_state
                .to_updated(block_number, event_type)
                .map(Into::into),
            Self::Inactive(inactive_state) => inactive_state
                .to_active(block_number, event_type)
                .map(Into::into),
            Self::Withdrawn(_) => Err(ProviderStateError::InvalidTransition),
        }
    }

    pub fn to_withdrawn(
        self,
        block_number: BlockNumber,
        event_type: EventType,
        service_params: &ServiceParameters,
    ) -> Result<Self, ProviderStateError> {
        match self {
            Self::New(_) => Err(ProviderStateError::InvalidTransition),
            Self::Active(active_state) => active_state
                .to_withdrawn(block_number, event_type, service_params)
                .map(Into::into),
            Self::Inactive(inactive_state) => inactive_state
                .to_withdrawn(block_number, event_type, service_params)
                .map(Into::into),
            // Withdrawn rewarded state can only transition from withdrawn state and it's only
            // allowed to transition if the withdrawn timestamp matches current block, in other
            // words, withdrawal can only be rewarded during withdrawal transaction.
            Self::Withdrawn(withdrawn_state) => withdrawn_state
                .to_withdrawn_rewarded(block_number, event_type)
                .map(Into::into),
        }
    }
}

impl From<ActiveState> for ProviderState {
    fn from(state: ActiveState) -> Self {
        Self::Active(state)
    }
}

impl From<InactiveState> for ProviderState {
    fn from(state: InactiveState) -> Self {
        Self::Inactive(state)
    }
}

impl From<WithdrawnState> for ProviderState {
    fn from(state: WithdrawnState) -> Self {
        Self::Withdrawn(state)
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{PUBLIC_KEY_LENGTH, VerifyingKey};

    use super::*;

    fn default_service_params() -> ServiceParameters {
        ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 100,
            reward_contract: [0; 32],
            timestamp: 0,
        }
    }

    fn gen_provider_id() -> ProviderId {
        let public_key_bytes: [u8; PUBLIC_KEY_LENGTH] = [
            215, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211, 201, 100, 7, 58, 14, 225, 114,
            243, 218, 166, 35, 37, 175, 2, 26, 104, 247, 7, 81, 26,
        ];

        VerifyingKey::from_bytes(&public_key_bytes).unwrap()
    }

    #[test]
    fn test_new_to_active() {
        let provider_id = gen_provider_id();
        let declaration_id = [1; 32];

        let initial_state = ProviderState::new(0, provider_id, declaration_id);
        let active_state = initial_state.to_active(0, EventType::Declaration).unwrap();

        if let ProviderState::Active(active_state) = active_state {
            assert_eq!(active_state.0.declaration_id, declaration_id);
            assert_eq!(active_state.0.created, 0);
        } else {
            panic!("Failed to transition to active state");
        }
    }

    #[test]
    fn test_active_to_reward_to_withdraw() {
        let provider_id = gen_provider_id();
        let declaration_id = [1; 32];
        let service_params = default_service_params();

        let active_state = ProviderState::new(0, provider_id, declaration_id)
            .to_active(0, EventType::Declaration)
            .unwrap();

        let rewarded_state = active_state.to_active(5, EventType::Reward).unwrap();

        let withdrawn_state = rewarded_state
            .to_withdrawn(11, EventType::Withdrawal, &service_params)
            .unwrap();

        // Withdrawn can't transition to active.
        let withdrawn_state = withdrawn_state.to_active(15, EventType::Declaration);
        assert!(withdrawn_state.is_err());
    }

    #[test]
    fn test_withdrawal_constraints() {
        let provider_id = gen_provider_id();
        let declaration_id = [1; 32];
        let service_params = default_service_params();

        let active_state = ProviderState::new(0, provider_id, declaration_id)
            .to_active(0, EventType::Declaration)
            .unwrap();

        // Withdrawal before lock period should fail.
        let early_withdrawal = active_state.to_withdrawn(5, EventType::Withdrawal, &service_params);
        assert!(early_withdrawal.is_err());

        let active_state = ProviderState::new(0, provider_id, declaration_id)
            .to_active(0, EventType::Declaration)
            .unwrap();

        // Withdrawal after lock period should succeed.
        let late_withdrawal = active_state
            .to_withdrawn(15, EventType::Withdrawal, &service_params)
            .unwrap();

        // Withdrawn state can only be rewarded in the same block.
        let rewarded_withdrawal = late_withdrawal
            .to_withdrawn(15, EventType::Reward, &service_params)
            .unwrap();

        // Withdrawal can't be rewarded in future.
        let reward_withdrawal_different_block =
            rewarded_withdrawal.to_withdrawn(16, EventType::Reward, &service_params);

        assert!(reward_withdrawal_different_block.is_err());
    }

    #[test]
    fn test_inactive_to_active_to_withdraw() {
        let provider_id = gen_provider_id();
        let declaration_id = [1; 32];
        let service_params = default_service_params();

        let provider_info = ProviderInfo {
            provider_id,
            declaration_id,
            created: 0,
            rewarded: None,
            withdrawn: None,
        };

        let inactive_state = ProviderState::from_info(100, provider_info, &service_params);
        assert!(matches!(inactive_state, ProviderState::Inactive(_)));

        // Inactive state can't declare a service.
        let active_state = inactive_state.to_active(100, EventType::Declaration);
        assert!(active_state.is_err());

        let inactive_state = ProviderState::from_info(100, provider_info, &service_params);
        let withdrawn_state =
            inactive_state.to_withdrawn(115, EventType::Withdrawal, &service_params);

        assert!(withdrawn_state.is_ok());
    }
}
