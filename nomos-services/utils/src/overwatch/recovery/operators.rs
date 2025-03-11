use std::fmt::Debug;

use log::error;
use overwatch::services::state::{ServiceState, StateOperator};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    overwatch::recovery::{errors::RecoveryError, RecoveryResult},
    traits::FromSettings,
};

pub trait RecoveryBackend {
    type State: ServiceState;
    fn load_state(&self) -> RecoveryResult<Self::State>;
    fn save_state(&self, state: &Self::State) -> RecoveryResult<()>;
}

#[derive(Debug, Clone)]
pub struct RecoveryOperator<Backend> {
    recovery_backend: Backend,
}

impl<Backend> RecoveryOperator<Backend>
where
    Backend: RecoveryBackend,
{
    const fn new(recovery_backend: Backend) -> Self {
        Self { recovery_backend }
    }
}

#[async_trait::async_trait]
impl<Backend> StateOperator for RecoveryOperator<Backend>
where
    Backend: RecoveryBackend
        + FromSettings<Settings = <Backend::State as ServiceState>::Settings>
        + Send,
    Backend::State: Serialize + DeserializeOwned + Send,
{
    type StateInput = Backend::State;
    type Settings = Backend::Settings;
    type LoadError = RecoveryError;

    fn try_load(
        settings: &<Self::StateInput as ServiceState>::Settings,
    ) -> Result<Option<Self::StateInput>, Self::LoadError> {
        Backend::from_settings(settings)
            .load_state()
            .map(Option::from)
    }

    fn from_settings(settings: <Self::StateInput as ServiceState>::Settings) -> Self {
        let recovery_backend = Backend::from_settings(&settings);
        Self::new(recovery_backend)
    }

    async fn run(&mut self, state: Self::StateInput) {
        let save_result = self.recovery_backend.save_state(&state);
        if let Err(error) = save_result {
            error!("{}", error);
        }
    }
}
