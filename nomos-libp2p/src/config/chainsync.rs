use serde::{Deserialize, Serialize};

/// Chainsync configuration settings. When set to `None`, chainsync is disabled.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings;
