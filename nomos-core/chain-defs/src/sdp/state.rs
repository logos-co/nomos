use std::cmp::max;

use thiserror::Error;

use super::{DeclarationState, EventType, ServiceParameters};
use crate::block::BlockNumber;

#[derive(Error, Clone, Debug, PartialEq, Eq)]
pub enum ActiveStateError {
    #[error("Active declaration state can happen only when provider is created")]
    ActiveNotOnCreated,
    #[error("Active state can not be updated to active state during withdrawal event")]
    ActiveDuringWithdrawal,
    #[error("Locked period did not pass yet")]
    WithdrawalWhileLocked,
    #[error("Active can not transition to withdrawn during {0:?} event")]
    WithdrawalInvalidEvent(EventType),
}

#[derive(Debug, Eq, PartialEq)]
pub struct ActiveState<'a>(&'a mut DeclarationState);

impl<'a> ActiveState<'a> {
    const fn try_into_updated(self, block_number: BlockNumber) -> Result<Self, ActiveStateError> {
        self.0.active = block_number;
        Ok(self)
    }

    const fn try_into_withdrawn(
        self,
        block_number: BlockNumber,
        service_params: &'_ ServiceParameters,
    ) -> Result<WithdrawnState<'a>, ActiveStateError> {
        if self.0.created.wrapping_add(service_params.lock_period) >= block_number {
            return Err(ActiveStateError::WithdrawalWhileLocked);
        }
        self.0.withdrawn = Some(block_number);
        Ok(WithdrawnState::<'a>(self.0))
    }
}

#[derive(Error, Clone, Debug, PartialEq, Eq)]
pub enum InactiveStateError {
    #[error("Inactive can not transition to active during {0:?} event")]
    ActiveInvalidEvent(EventType),
    #[error("Locked period did not pass yet")]
    WithdrawalWhileLocked,
    #[error("Inactive can not transition to withdrawn during {0:?} event")]
    WithdrawalInvalidEvent(EventType),
}

#[derive(Debug, Eq, PartialEq)]
pub struct InactiveState<'a>(&'a mut DeclarationState);

impl<'a> InactiveState<'a> {
    const fn try_into_active(
        self,
        block_number: BlockNumber,
    ) -> Result<ActiveState<'a>, InactiveStateError> {
        self.0.active = block_number;
        Ok(ActiveState(self.0))
    }

    const fn try_into_withdrawn(
        self,
        block_number: BlockNumber,
        service_params: &ServiceParameters,
    ) -> Result<WithdrawnState<'a>, InactiveStateError> {
        if self.0.created.wrapping_add(service_params.lock_period) >= block_number {
            return Err(InactiveStateError::WithdrawalWhileLocked);
        }
        self.0.withdrawn = Some(block_number);
        Ok(WithdrawnState(self.0))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct WithdrawnState<'a>(&'a mut DeclarationState);

#[derive(Error, Clone, PartialEq, Eq, Debug)]
pub enum DeclarationStateError {
    #[error(transparent)]
    Active(#[from] ActiveStateError),
    #[error(transparent)]
    Inactive(#[from] InactiveStateError),
    #[error("Withdrawn can not transition to any other state")]
    WithdrawnToOtherState,
    #[error("Provided block is in past, can not transition to state in previous block")]
    BlockFromPast,
}

#[derive(Debug, Eq, PartialEq)]
pub enum TransientDeclarationState<'a> {
    Active(ActiveState<'a>),
    Inactive(InactiveState<'a>),
    Withdrawn(WithdrawnState<'a>),
}

impl<'a> TransientDeclarationState<'a> {
    pub fn try_from_state(
        block_number: BlockNumber,
        declaration_state: &'a mut DeclarationState,
        service_params: &ServiceParameters,
    ) -> Result<Self, DeclarationStateError> {
        if declaration_state.created > block_number {
            return Err(DeclarationStateError::BlockFromPast);
        }

        if declaration_state.withdrawn.is_some() {
            return Ok(WithdrawnState(declaration_state).into());
        }

        // This section checks if recently created provider is still considered active
        // even without having activity recorded yet.
        if declaration_state
            .created
            .wrapping_add(service_params.inactivity_period)
            > block_number
        {
            return Ok(ActiveState(declaration_state).into());
        }

        // Check if provider has ever got the activity recorded first and then see if
        // the activity record was recent.
        let activity = max(declaration_state.active, declaration_state.created);
        if block_number.wrapping_sub(activity) <= service_params.inactivity_period {
            return Ok(ActiveState(declaration_state).into());
        }

        Ok(InactiveState(declaration_state).into())
    }

    #[must_use]
    fn last_block_number(&self) -> BlockNumber {
        let declaration_state: &DeclarationState = match self {
            Self::Active(active_state) => active_state.0,
            Self::Inactive(inactive_state) => inactive_state.0,
            Self::Withdrawn(withdrawn_state) => withdrawn_state.0,
        };
        if let Some(withdrawn_timestamp) = declaration_state.withdrawn {
            return withdrawn_timestamp;
        }
        max(declaration_state.active, declaration_state.created)
    }

    pub fn try_into_active(self, block_number: BlockNumber) -> Result<Self, DeclarationStateError> {
        if self.last_block_number() > block_number {
            return Err(DeclarationStateError::BlockFromPast);
        }
        match self {
            Self::Active(active_state) => active_state
                .try_into_updated(block_number)
                .map(Into::into)
                .map_err(DeclarationStateError::from),
            Self::Inactive(inactive_state) => inactive_state
                .try_into_active(block_number)
                .map(Into::into)
                .map_err(DeclarationStateError::from),
            Self::Withdrawn(_) => Err(DeclarationStateError::WithdrawnToOtherState),
        }
    }

    pub fn try_into_withdrawn(
        self,
        block_number: BlockNumber,
        service_params: &ServiceParameters,
    ) -> Result<Self, DeclarationStateError> {
        if self.last_block_number() > block_number {
            return Err(DeclarationStateError::BlockFromPast);
        }
        match self {
            Self::Active(active_state) => active_state
                .try_into_withdrawn(block_number, service_params)
                .map(Into::into)
                .map_err(DeclarationStateError::from),
            Self::Inactive(inactive_state) => inactive_state
                .try_into_withdrawn(block_number, service_params)
                .map(Into::into)
                .map_err(DeclarationStateError::from),
            Self::Withdrawn(_) => Err(DeclarationStateError::WithdrawnToOtherState),
        }
    }
}

impl<'a> From<ActiveState<'a>> for TransientDeclarationState<'a> {
    fn from(state: ActiveState<'a>) -> Self {
        Self::Active(state)
    }
}

impl<'a> From<InactiveState<'a>> for TransientDeclarationState<'a> {
    fn from(state: InactiveState<'a>) -> Self {
        Self::Inactive(state)
    }
}

impl<'a> From<WithdrawnState<'a>> for TransientDeclarationState<'a> {
    fn from(state: WithdrawnState<'a>) -> Self {
        Self::Withdrawn(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn default_service_params() -> ServiceParameters {
        ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 100,
            timestamp: 0,
        }
    }

    #[test]
    fn test_info_to_inactive_state() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(0);

        let inactive_state =
            TransientDeclarationState::try_from_state(21, &mut declaration_state, &service_params)
                .unwrap();
        assert!(matches!(
            inactive_state,
            TransientDeclarationState::Inactive(_)
        ));
    }

    #[test]
    fn test_info_to_inactive_active_state() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(100);
        declaration_state.active = 110;

        let inactive_activity_record_state =
            TransientDeclarationState::try_from_state(200, &mut declaration_state, &service_params)
                .unwrap();
        assert!(matches!(
            inactive_activity_record_state,
            TransientDeclarationState::Inactive(_)
        ));
        assert_eq!(inactive_activity_record_state.last_block_number(), 110);
    }

    #[test]
    fn test_info_to_active_state() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(100);

        let active_state =
            TransientDeclarationState::try_from_state(111, &mut declaration_state, &service_params)
                .unwrap();
        assert!(matches!(active_state, TransientDeclarationState::Active(_)));
    }

    #[test]
    fn test_info_to_active_recorded_state() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(100);
        declaration_state.active = 110;

        let active_recorded_state =
            TransientDeclarationState::try_from_state(111, &mut declaration_state, &service_params)
                .unwrap();
        assert!(matches!(
            active_recorded_state,
            TransientDeclarationState::Active(_)
        ));
        assert_eq!(active_recorded_state.last_block_number(), 110);
    }

    #[test]
    fn test_info_to_withdrawn_state() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(100);
        declaration_state.active = 111;
        declaration_state.withdrawn = Some(121);

        let withdrawn_state =
            TransientDeclarationState::try_from_state(131, &mut declaration_state, &service_params)
                .unwrap();
        assert!(matches!(
            withdrawn_state,
            TransientDeclarationState::Withdrawn(_)
        ));
        assert_eq!(withdrawn_state.last_block_number(), 121);
    }

    #[test]
    fn test_previous_block() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(3);

        // Provider created in block 3, trying to convert to state in block 2.
        let res =
            TransientDeclarationState::try_from_state(2, &mut declaration_state, &service_params);
        assert!(matches!(res, Err(DeclarationStateError::BlockFromPast)));

        // Provider activity recorded in block 5, trying to withdraw in block 4.
        let active_state =
            TransientDeclarationState::try_from_state(3, &mut declaration_state, &service_params)
                .unwrap();
        let active_recorded = active_state.try_into_active(5).unwrap();
        let res = active_recorded.try_into_withdrawn(4, &service_params);
        assert!(matches!(res, Err(DeclarationStateError::BlockFromPast)));
    }

    #[test]
    fn test_active_to_active_declaration() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(0);

        let active_state =
            TransientDeclarationState::try_from_state(0, &mut declaration_state, &service_params)
                .unwrap();
        let active_state = active_state.try_into_active(0).unwrap();

        if let TransientDeclarationState::Active(active_state) = active_state {
            assert_eq!(active_state.0.created, 0);
        } else {
            panic!("Failed to transition to active state");
        }
    }

    #[test]
    fn test_active_to_activity_recorded_to_withdraw() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(0);

        let active_state =
            TransientDeclarationState::try_from_state(0, &mut declaration_state, &service_params)
                .unwrap();
        let active_recorded = active_state.try_into_active(5).unwrap();
        let withdrawn_state = active_recorded
            .try_into_withdrawn(11, &service_params)
            .unwrap();

        // Withdrawn can't transition to active.
        let withdrawn_state = withdrawn_state.try_into_active(15);
        assert!(withdrawn_state.is_err());
    }

    #[test]
    fn test_withdrawal_constraints() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(0);

        let active_state =
            TransientDeclarationState::try_from_state(0, &mut declaration_state, &service_params)
                .unwrap();

        // Withdrawal before lock period should fail.
        let early_withdrawal = active_state.try_into_withdrawn(5, &service_params);
        assert!(early_withdrawal.is_err());

        // States are consumed, to continue the test we need to create a new state.
        let active_state =
            TransientDeclarationState::try_from_state(0, &mut declaration_state, &service_params)
                .unwrap()
                .try_into_active(0)
                .unwrap();

        // Withdrawal after lock period should succeed.
        let late_withdrawal = active_state
            .try_into_withdrawn(15, &service_params)
            .unwrap();

        // Withdrawn state can not have activity recorded in the same block.
        let res = late_withdrawal.try_into_withdrawn(15, &service_params);
        assert!(matches!(
            res,
            Err(DeclarationStateError::WithdrawnToOtherState)
        ));

        // Withdrawal can't be activity recorded in future.
        let mut declaration_state = DeclarationState::new(0);
        let active_withdrawal =
            TransientDeclarationState::try_from_state(0, &mut declaration_state, &service_params)
                .unwrap()
                .try_into_withdrawn(11, &service_params)
                .unwrap();

        let active_withdrawal_different_block =
            active_withdrawal.try_into_withdrawn(16, &service_params);

        assert!(active_withdrawal_different_block.is_err());
    }

    #[test]
    fn test_inactive_to_withdraw() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(0);

        // Try to make inactive state then withdraw.
        let inactive_state =
            TransientDeclarationState::try_from_state(100, &mut declaration_state, &service_params)
                .unwrap();

        let withdrawn_state = inactive_state.try_into_withdrawn(115, &service_params);

        assert!(withdrawn_state.is_ok());
    }

    #[test]
    fn test_inactive_to_active_to_withdraw() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(0);

        // Try to make inactive state active again and then withdraw.
        let inactive_state =
            TransientDeclarationState::try_from_state(100, &mut declaration_state, &service_params)
                .unwrap();

        let active_state = inactive_state.try_into_active(105).unwrap();

        let withdrawn_state = active_state.try_into_withdrawn(115, &service_params);

        assert!(withdrawn_state.is_ok());
    }

    #[test]
    fn test_withdrawn_cannot_transition_to_inactive() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(0);

        let active_state =
            TransientDeclarationState::try_from_state(0, &mut declaration_state, &service_params)
                .unwrap()
                .try_into_active(0)
                .unwrap();

        let withdrawn_state = active_state
            .try_into_withdrawn(15, &service_params)
            .unwrap();

        // Withdrawn should not be able to transition back to Inactive.
        let inactive_state = withdrawn_state.try_into_active(20);
        assert!(inactive_state.is_err());
    }

    #[test]
    fn test_inactive_cannot_withdraw_before_lock_period() {
        let service_params = ServiceParameters {
            lock_period: 20,
            inactivity_period: 5,
            retention_period: 30,
            timestamp: 0,
        };
        let mut declaration_state = DeclarationState::new(0);

        let inactive_state =
            TransientDeclarationState::try_from_state(10, &mut declaration_state, &service_params)
                .unwrap();
        assert!(matches!(
            inactive_state,
            TransientDeclarationState::Inactive(_)
        ));

        // Withdrawal should fail before the lock period is over.
        let withdrawal_attempt = inactive_state.try_into_withdrawn(15, &service_params);
        assert!(withdrawal_attempt.is_err());
    }

    #[test]
    fn test_active_cannot_record_activity_directly_when_withdrawing() {
        let service_params = default_service_params();
        let mut declaration_state = DeclarationState::new(0);

        let active_state =
            TransientDeclarationState::try_from_state(0, &mut declaration_state, &service_params)
                .unwrap()
                .try_into_active(0)
                .unwrap();

        // Attempt to transition to active recorded state without an intermediate step.
        let active_recorded_state = active_state.try_into_withdrawn(5, &service_params);
        assert!(active_recorded_state.is_err());
    }
}
