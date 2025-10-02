use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use services_utils::overwatch::recovery::backends::FileBackendSettings;

/// Settings for the tx mempool service.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TxMempoolSettings<PoolSettings, NetworkAdapterSettings, ProcessorSettings> {
    /// The mempool settings.
    pub pool: PoolSettings,
    /// The network adapter settings.
    pub network_adapter: NetworkAdapterSettings,
    /// The mempool payload processor settings.
    pub processor: ProcessorSettings,
    /// The recovery file path, for the service's [`RecoveryOperator`].
    pub recovery_path: PathBuf,
}

impl<PoolSettings, NetworkAdapterSettings, ProcessorSettings> FileBackendSettings
    for TxMempoolSettings<PoolSettings, NetworkAdapterSettings, ProcessorSettings>
{
    fn recovery_file(&self) -> &PathBuf {
        &self.recovery_path
    }
}
