use std::{convert::Infallible, marker::PhantomData};

use overwatch::services::state::ServiceState;
use serde::{Deserialize, Serialize};

use crate::TxMempoolSettings;

/// State that is maintained across service restarts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxMempoolState<PoolState, PoolSettings, NetworkSettings, ProcessorSettings> {
    /// The (optional) pool snapshot.
    pub(crate) pool: Option<PoolState>,
    #[serde(skip)]
    _phantom: PhantomData<(PoolSettings, NetworkSettings, ProcessorSettings)>,
}

impl<PoolState, PoolSettings, NetworkSettings, ProcessorSettings>
    TxMempoolState<PoolState, PoolSettings, NetworkSettings, ProcessorSettings>
{
    pub const fn pool(&self) -> Option<&PoolState> {
        self.pool.as_ref()
    }
}

impl<PoolState, PoolSettings, NetworkSettings, ProcessorSettings> From<PoolState>
    for TxMempoolState<PoolState, PoolSettings, NetworkSettings, ProcessorSettings>
{
    fn from(value: PoolState) -> Self {
        Self {
            pool: Some(value),
            _phantom: PhantomData,
        }
    }
}

impl<PoolState, PoolSettings, NetworkSettings, ProcessorSettings> ServiceState
    for TxMempoolState<PoolState, PoolSettings, NetworkSettings, ProcessorSettings>
{
    type Error = Infallible;
    type Settings = TxMempoolSettings<PoolSettings, NetworkSettings, ProcessorSettings>;

    fn from_settings(_settings: &Self::Settings) -> Result<Self, Self::Error> {
        Ok(Self {
            pool: None,
            _phantom: PhantomData,
        })
    }
}
