use core::num::NonZeroUsize;

use lb_services_utils::overwatch::{RecoveryData, StorageRecoverySettings};
use serde::{Deserialize, Serialize};

pub const RECOVERY_KEY_SUFFIX: &[u8] = b"mempool";

/// Settings for the tx mempool service.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxMempoolSettings<PoolSettings, NetworkAdapterSettings> {
    /// The mempool settings.
    pub pool: PoolSettings,
    /// The network adapter settings.
    pub network_adapter: NetworkAdapterSettings,
    pub seen_items_subscription_channel_size: NonZeroUsize,
    #[serde(skip)]
    pub recovery_data: RecoveryData,
}

impl<PoolSettings, NetworkAdapterSettings> StorageRecoverySettings
    for TxMempoolSettings<PoolSettings, NetworkAdapterSettings>
{
    const RECOVERY_KEY_SUFFIX: &'static [u8] = RECOVERY_KEY_SUFFIX;

    fn recovery_data(&self) -> &RecoveryData {
        &self.recovery_data
    }
}
