use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};
use services_utils::overwatch::recovery::backends::FileBackendSettings;

/// Settings for the DA mempool service.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DaMempoolSettings<PoolSettings, NetworkAdapterSettings> {
    /// The mempool settings.
    pub pool: PoolSettings,
    /// The network adapter settings.
    pub network_adapter: NetworkAdapterSettings,
    /// The recovery file path, for the service's [`RecoveryOperator`].
    pub recovery_path: PathBuf,
    /// Trigger sampling delay.
    pub trigger_sampling_delay: Duration,
}

impl<PoolSettings, NetworkAdapterSettings> FileBackendSettings
    for DaMempoolSettings<PoolSettings, NetworkAdapterSettings>
{
    fn recovery_file(&self) -> &PathBuf {
        &self.recovery_path
    }
}
