use thiserror::Error;

use crate::{BlockNumber, DeclarationId, EventType, ProviderInfo, ServiceParameters};

#[derive(Error, Debug)]
pub enum ActiveStateError {
    #[error("Active declaration state can happen only when provider is created")]
    ToActiveNotOnCreated,
    #[error("Active state can not be updated to active state during withdrawal event")]
    ToActiveDuringWithdrawal,
    #[error("Locked period did not pass yet")]
    ToWithdrawalWhileLocked,
    #[error("Active can not transition to withdrawn during {0:?} event")]
    ToWithdrawalInvalidEvent(EventType),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ActiveState(ProviderInfo);

impl ActiveState {
    const fn try_into_updated(
        mut self,
        block_number: BlockNumber,
        event_type: EventType,
    ) -> Result<Self, ActiveStateError> {
        match event_type {
            EventType::Declaration => {
                if self.0.created == block_number {
                    Ok(self)
                } else {
                    Err(ActiveStateError::ToActiveNotOnCreated)
                }
            }
            EventType::Reward => {
                self.0.rewarded = Some(block_number);
                Ok(self)
            }
            EventType::Withdrawal => Err(ActiveStateError::ToActiveDuringWithdrawal),
        }
    }

    const fn try_into_withdrawn<ContractAddress>(
        mut self,
        block_number: BlockNumber,
        event_type: EventType,
        service_params: &ServiceParameters<ContractAddress>,
    ) -> Result<WithdrawnState, ActiveStateError> {
        match event_type {
            EventType::Withdrawal => {
                if self.0.created.wrapping_add(service_params.lock_period) >= block_number {
                    return Err(ActiveStateError::ToWithdrawalWhileLocked);
                }
                self.0.withdrawn = Some(block_number);
                Ok(WithdrawnState(self.0))
            }
            EventType::Declaration | EventType::Reward => {
                Err(ActiveStateError::ToWithdrawalInvalidEvent(event_type))
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum InactiveStateError {
    #[error("Inactive can not transition to active during {0:?} event")]
    ToActiveInvalidEvent(EventType),
    #[error("Locked period did not pass yet")]
    ToWithdrawalWhileLocked,
    #[error("Inactive can not transition to withdrawn during {0:?} event")]
    ToWithdrawalInvalidEvent(EventType),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct InactiveState(ProviderInfo);

impl InactiveState {
    const fn try_into_active(
        mut self,
        block_number: BlockNumber,
        event_type: EventType,
    ) -> Result<ActiveState, InactiveStateError> {
        match event_type {
            EventType::Reward => {
                self.0.rewarded = Some(block_number);
                Ok(ActiveState(self.0))
            }
            EventType::Declaration | EventType::Withdrawal => {
                Err(InactiveStateError::ToActiveInvalidEvent(event_type))
            }
        }
    }

    const fn try_into_withdrawn<ContractAddress>(
        mut self,
        block_number: BlockNumber,
        event_type: EventType,
        service_params: &ServiceParameters<ContractAddress>,
    ) -> Result<WithdrawnState, InactiveStateError> {
        match event_type {
            EventType::Withdrawal => {
                if self.0.created.wrapping_add(service_params.lock_period) >= block_number {
                    return Err(InactiveStateError::ToWithdrawalWhileLocked);
                }
                self.0.withdrawn = Some(block_number);
                Ok(WithdrawnState(self.0))
            }
            EventType::Declaration | EventType::Reward => {
                Err(InactiveStateError::ToWithdrawalInvalidEvent(event_type))
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum WithdrawnStateError {
    #[error("Withdawn state can be rewarded only during the same block")]
    ToWithdrawnRewardedNotSameBlock,
    #[error("Withdrawn can not transition to withdrawn rewarded during {0:?} event")]
    ToWithdrawnRewardedInvalidEvent(EventType),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct WithdrawnState(ProviderInfo);

impl WithdrawnState {
    fn try_into_withdrawn_rewarded(
        mut self,
        block_number: BlockNumber,
        event_type: EventType,
    ) -> Result<Self, WithdrawnStateError> {
        if matches!(event_type, EventType::Reward) {
            // Only allow reward for withdrawal in the same block number.
            if self.0.withdrawn != Some(block_number) {
                return Err(WithdrawnStateError::ToWithdrawnRewardedNotSameBlock);
            }

            self.0.rewarded = Some(block_number);
            Ok(self)
        } else {
            Err(WithdrawnStateError::ToWithdrawnRewardedInvalidEvent(
                event_type,
            ))
        }
    }
}

#[derive(Error, Debug)]
pub enum ProviderStateError {
    #[error(transparent)]
    Active(#[from] ActiveStateError),
    #[error(transparent)]
    Inactive(#[from] InactiveStateError),
    #[error(transparent)]
    Withdrawn(#[from] WithdrawnStateError),
    #[error("Withdrawn can not transition to active")]
    WithdrawnToActive,
    #[error("Provided block is in past, can not transition to state in previous block")]
    BlockFromPast,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum ProviderState {
    Active(ActiveState),
    Inactive(InactiveState),
    Withdrawn(WithdrawnState),
}

impl From<ProviderState> for ProviderInfo {
    fn from(state: ProviderState) -> Self {
        match state {
            ProviderState::Active(active_state) => active_state.0,
            ProviderState::Inactive(inactive_state) => inactive_state.0,
            ProviderState::Withdrawn(withdrawn_state) => withdrawn_state.0,
        }
    }
}

impl ProviderState {
    pub fn try_from_info<ConstractAddress>(
        block_number: BlockNumber,
        provider_info: &ProviderInfo,
        service_params: &ServiceParameters<ConstractAddress>,
    ) -> Result<Self, ProviderStateError> {
        if provider_info.created > block_number {
            return Err(ProviderStateError::BlockFromPast);
        }

        if provider_info.withdrawn.is_some() {
            return Ok(WithdrawnState(*provider_info).into());
        }

        // This section checks if recently created provider is still considered active
        // even without having requested a reward yet.
        if provider_info
            .created
            .wrapping_add(service_params.inactivity_period)
            > block_number
        {
            return Ok(ActiveState(*provider_info).into());
        }

        // Check if provider has ever got the reward first and then see if the reward
        // request was recent.
        if let Some(rewarded) = provider_info.rewarded {
            if block_number.wrapping_sub(rewarded) <= service_params.inactivity_period {
                return Ok(ActiveState(*provider_info).into());
            }
        }

        Ok(InactiveState(*provider_info).into())
    }

    #[must_use]
    const fn last_block_number(&self) -> BlockNumber {
        let provider_info: ProviderInfo = match self {
            Self::Active(active_state) => active_state.0,
            Self::Inactive(inactive_state) => inactive_state.0,
            Self::Withdrawn(withdrawn_state) => withdrawn_state.0,
        };
        if let Some(withdrawn_timestamp) = provider_info.withdrawn {
            return withdrawn_timestamp;
        }
        if let Some(rewards_timestamp) = provider_info.rewarded {
            return rewards_timestamp;
        }
        provider_info.created
    }

    #[must_use]
    pub const fn declaration_id(&self) -> DeclarationId {
        match self {
            Self::Active(active_state) => active_state.0.declaration_id,
            Self::Inactive(inactive_state) => inactive_state.0.declaration_id,
            Self::Withdrawn(withdrawn_state) => withdrawn_state.0.declaration_id,
        }
    }

    pub fn try_into_active(
        self,
        block_number: BlockNumber,
        event_type: EventType,
    ) -> Result<Self, ProviderStateError> {
        if self.last_block_number() > block_number {
            return Err(ProviderStateError::BlockFromPast);
        }
        match self {
            Self::Active(active_state) => active_state
                .try_into_updated(block_number, event_type)
                .map(Into::into)
                .map_err(ProviderStateError::from),
            Self::Inactive(inactive_state) => inactive_state
                .try_into_active(block_number, event_type)
                .map(Into::into)
                .map_err(ProviderStateError::from),
            Self::Withdrawn(_) => Err(ProviderStateError::WithdrawnToActive),
        }
    }

    pub fn try_into_withdrawn<ContractAddress>(
        self,
        block_number: BlockNumber,
        event_type: EventType,
        service_params: &ServiceParameters<ContractAddress>,
    ) -> Result<Self, ProviderStateError> {
        if self.last_block_number() > block_number {
            return Err(ProviderStateError::BlockFromPast);
        }
        match self {
            Self::Active(active_state) => active_state
                .try_into_withdrawn(block_number, event_type, service_params)
                .map(Into::into)
                .map_err(ProviderStateError::from),
            Self::Inactive(inactive_state) => inactive_state
                .try_into_withdrawn(block_number, event_type, service_params)
                .map(Into::into)
                .map_err(ProviderStateError::from),
            // Withdrawn rewarded state can only transition from withdrawn state and it's only
            // allowed to transition if the withdrawn timestamp matches current block, in other
            // words, withdrawal can only be rewarded in withdrawal block.
            Self::Withdrawn(withdrawn_state) => withdrawn_state
                .try_into_withdrawn_rewarded(block_number, event_type)
                .map(Into::into)
                .map_err(ProviderStateError::from),
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
    use super::*;
    use crate::ProviderId;

    type MockContractAddress = [u8; 32];

    const fn default_service_params() -> ServiceParameters<MockContractAddress> {
        ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 100,
            reward_contract: [0; 32],
            timestamp: 0,
        }
    }

    #[test]
    fn test_info_to_inactive_state() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let provider_info = ProviderInfo::new(0, provider_id, declaration_id);

        let inactive_state =
            ProviderState::try_from_info(21, &provider_info, &service_params).unwrap();
        assert!(matches!(inactive_state, ProviderState::Inactive(_)));
    }

    #[test]
    fn test_info_to_inactive_rewarded_state() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let mut provider_info = ProviderInfo::new(100, provider_id, declaration_id);
        provider_info.rewarded = Some(110);

        let inactive_rewarded_state =
            ProviderState::try_from_info(200, &provider_info, &service_params).unwrap();
        assert!(matches!(
            inactive_rewarded_state,
            ProviderState::Inactive(_)
        ));
        assert_eq!(inactive_rewarded_state.last_block_number(), 110);
    }

    #[test]
    fn test_info_to_active_state() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let provider_info = ProviderInfo::new(100, provider_id, declaration_id);

        let active_state =
            ProviderState::try_from_info(111, &provider_info, &service_params).unwrap();
        assert!(matches!(active_state, ProviderState::Active(_)));
    }

    #[test]
    fn test_info_to_active_rewarded_state() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let mut provider_info = ProviderInfo::new(100, provider_id, declaration_id);
        provider_info.rewarded = Some(110);

        let active_rewarded_state =
            ProviderState::try_from_info(111, &provider_info, &service_params).unwrap();
        assert!(matches!(active_rewarded_state, ProviderState::Active(_)));
        assert_eq!(active_rewarded_state.last_block_number(), 110);
    }

    #[test]
    fn test_info_to_withdrawn_state() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let mut provider_info = ProviderInfo::new(100, provider_id, declaration_id);
        provider_info.rewarded = Some(111);
        provider_info.withdrawn = Some(121);

        let withdrawn_state =
            ProviderState::try_from_info(131, &provider_info, &service_params).unwrap();
        assert!(matches!(withdrawn_state, ProviderState::Withdrawn(_)));
        assert_eq!(withdrawn_state.last_block_number(), 121);
    }

    #[test]
    fn test_previous_block() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let provider_info = ProviderInfo::new(3, provider_id, declaration_id);

        // Provider created in block 3, trying to convert to state in block 2.
        let res = ProviderState::try_from_info(2, &provider_info, &service_params);
        assert!(matches!(res, Err(ProviderStateError::BlockFromPast)));

        // Provider rewarded in block 5, trying to withdraw in block 4.
        let active_state =
            ProviderState::try_from_info(3, &provider_info, &service_params).unwrap();
        let rewarded_state = active_state.try_into_active(5, EventType::Reward).unwrap();
        let res = rewarded_state.try_into_withdrawn(4, EventType::Withdrawal, &service_params);
        assert!(matches!(res, Err(ProviderStateError::BlockFromPast)));
    }

    #[test]
    fn test_active_to_active_declaration() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let provider_info = ProviderInfo::new(0, provider_id, declaration_id);

        let active_state =
            ProviderState::try_from_info(0, &provider_info, &service_params).unwrap();
        let active_state = active_state
            .try_into_active(0, EventType::Declaration)
            .unwrap();

        if let ProviderState::Active(active_state) = active_state {
            assert_eq!(active_state.0.declaration_id, declaration_id);
            assert_eq!(active_state.0.created, 0);
        } else {
            panic!("Failed to transition to active state");
        }
    }

    #[test]
    fn test_active_to_reward_to_withdraw() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let provider_info = ProviderInfo::new(0, provider_id, declaration_id);

        let active_state =
            ProviderState::try_from_info(0, &provider_info, &service_params).unwrap();
        let rewarded_state = active_state.try_into_active(5, EventType::Reward).unwrap();
        let withdrawn_state = rewarded_state
            .try_into_withdrawn(11, EventType::Withdrawal, &service_params)
            .unwrap();

        // Withdrawn can't transition to active.
        let withdrawn_state = withdrawn_state.try_into_active(15, EventType::Declaration);
        assert!(withdrawn_state.is_err());
    }

    #[test]
    fn test_withdrawal_constraints() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let provider_info = ProviderInfo::new(0, provider_id, declaration_id);

        let active_state =
            ProviderState::try_from_info(0, &provider_info, &service_params).unwrap();

        // Withdrawal before lock period should fail.
        let early_withdrawal =
            active_state.try_into_withdrawn(5, EventType::Withdrawal, &service_params);
        assert!(early_withdrawal.is_err());

        // States are consumed, to continue the test we need to create a new state.
        let active_state = ProviderState::try_from_info(0, &provider_info, &service_params)
            .unwrap()
            .try_into_active(0, EventType::Declaration)
            .unwrap();

        // Withdrawal after lock period should succeed.
        let late_withdrawal = active_state
            .try_into_withdrawn(15, EventType::Withdrawal, &service_params)
            .unwrap();

        // Withdrawn state can only be rewarded in the same block.
        let rewarded_withdrawal = late_withdrawal
            .try_into_withdrawn(15, EventType::Reward, &service_params)
            .unwrap();

        // Withdrawal can't be rewarded in future.
        let reward_withdrawal_different_block =
            rewarded_withdrawal.try_into_withdrawn(16, EventType::Reward, &service_params);

        assert!(reward_withdrawal_different_block.is_err());
    }

    #[test]
    fn test_inactive_to_withdraw() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();

        let provider_info = ProviderInfo {
            provider_id,
            declaration_id,
            created: 0,
            rewarded: None,
            withdrawn: None,
        };

        let inactive_state =
            ProviderState::try_from_info(100, &provider_info, &service_params).unwrap();
        assert!(matches!(inactive_state, ProviderState::Inactive(_)));

        // Inactive state can't declare a service.
        let active_state = inactive_state.try_into_active(100, EventType::Declaration);
        assert!(active_state.is_err());

        // Try to make inactive state then withdraw.
        let inactive_state =
            ProviderState::try_from_info(100, &provider_info, &service_params).unwrap();

        let withdrawn_state =
            inactive_state.try_into_withdrawn(115, EventType::Withdrawal, &service_params);

        assert!(withdrawn_state.is_ok());
    }

    #[test]
    fn test_inactive_to_active_to_withdraw() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();

        let provider_info = ProviderInfo {
            provider_id,
            declaration_id,
            created: 0,
            rewarded: None,
            withdrawn: None,
        };

        // Try to make inactive state active again and then withdraw.
        let inactive_state =
            ProviderState::try_from_info(100, &provider_info, &service_params).unwrap();

        let active_state = inactive_state
            .try_into_active(105, EventType::Reward)
            .unwrap();

        let withdrawn_state =
            active_state.try_into_withdrawn(115, EventType::Withdrawal, &service_params);

        assert!(withdrawn_state.is_ok());
    }

    #[test]
    fn test_withdrawn_cannot_transition_to_inactive() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let provider_info = ProviderInfo::new(0, provider_id, declaration_id);

        let active_state = ProviderState::try_from_info(0, &provider_info, &service_params)
            .unwrap()
            .try_into_active(0, EventType::Declaration)
            .unwrap();

        let withdrawn_state = active_state
            .try_into_withdrawn(15, EventType::Withdrawal, &service_params)
            .unwrap();

        // Withdrawn should not be able to transition back to Inactive.
        let inactive_state = withdrawn_state.try_into_active(20, EventType::Declaration);
        assert!(inactive_state.is_err());
    }

    #[test]
    fn test_inactive_cannot_withdraw_before_lock_period() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = ServiceParameters {
            lock_period: 20,
            inactivity_period: 5,
            retention_period: 30,
            reward_contract: [0; 32],
            timestamp: 0,
        };
        let provider_info = ProviderInfo::new(0, provider_id, declaration_id);

        let inactive_state =
            ProviderState::try_from_info(10, &provider_info, &service_params).unwrap();
        assert!(matches!(inactive_state, ProviderState::Inactive(_)));

        // Withdrawal should fail before the lock period is over.
        let withdrawal_attempt =
            inactive_state.try_into_withdrawn(15, EventType::Withdrawal, &service_params);
        assert!(withdrawal_attempt.is_err());
    }

    #[test]
    fn test_active_cannot_reward_directly_when_withdrawing() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let provider_info = ProviderInfo::new(0, provider_id, declaration_id);

        let active_state = ProviderState::try_from_info(0, &provider_info, &service_params)
            .unwrap()
            .try_into_active(0, EventType::Declaration)
            .unwrap();

        // Attempt to transition to rewarded state without an intermediate step.
        let rewarded_state = active_state.try_into_withdrawn(5, EventType::Reward, &service_params);
        assert!(rewarded_state.is_err());
    }

    #[test]
    fn test_invalid_event_transitions() {
        let provider_id = ProviderId([0; 32]);
        let declaration_id = DeclarationId([1; 32]);
        let service_params = default_service_params();
        let provider_info = ProviderInfo::new(0, provider_id, declaration_id);

        let active_state = ProviderState::try_from_info(0, &provider_info, &service_params)
            .unwrap()
            .try_into_active(0, EventType::Declaration)
            .unwrap();

        // Invalid event: trying to go from Active to Active with a non-declaration
        // event.
        let invalid_transition = active_state.try_into_active(5, EventType::Withdrawal);
        assert!(invalid_transition.is_err());

        let inactive_state =
            ProviderState::try_from_info(100, &provider_info, &service_params).unwrap();
        assert!(matches!(inactive_state, ProviderState::Inactive(_)));

        // Invalid event: Inactive cannot transition to Active with a withdrawal event.
        let invalid_to_active = inactive_state.try_into_active(105, EventType::Withdrawal);
        assert!(invalid_to_active.is_err());
    }
}
