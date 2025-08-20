use serde::{Deserialize, Serialize};

pub mod autonat_client;
pub mod mapping;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Settings {
    pub autonat: autonat_client::Settings,
    pub mapping: mapping::Settings,
    pub gateway_monitor: gateway_monitor::Settings,
}

pub mod gateway_monitor {
    use super::{Deserialize, Serialize};
    
    /// Gateway monitoring settings
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct Settings {
        /// How often to check for gateway address changes (in seconds)
        #[serde(default = "default_check_interval_secs")]
        pub check_interval_secs: u64,
        /// Timeout for gateway detection (in seconds)
        #[serde(default = "default_detection_timeout_secs")]
        pub detection_timeout_secs: u64,
        /// Whether gateway monitoring is enabled
        #[serde(default = "default_enabled")]
        pub enabled: bool,
    }

    const fn default_check_interval_secs() -> u64 {
        300 // 5 minutes
    }

    const fn default_detection_timeout_secs() -> u64 {
        10 // 10 seconds
    }

    const fn default_enabled() -> bool {
        true
    }

    impl Default for Settings {
        fn default() -> Self {
            Self {
                check_interval_secs: default_check_interval_secs(),
                detection_timeout_secs: default_detection_timeout_secs(),
                enabled: default_enabled(),
            }
        }
    }
}
