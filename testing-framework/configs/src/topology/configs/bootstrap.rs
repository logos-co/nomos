use std::time::Duration;

#[derive(Clone)]
pub struct GeneralBootstrapConfig {
    pub prolonged_bootstrap_period: Duration,
}

pub const SHORT_PROLONGED_BOOTSTRAP_PERIOD: Duration = Duration::from_secs(1);

#[must_use]
pub fn create_bootstrap_configs(
    ids: &[[u8; 32]],
    prolonged_bootstrap_period: Duration,
) -> Vec<GeneralBootstrapConfig> {
    ids.iter()
        .map(|_| GeneralBootstrapConfig {
            prolonged_bootstrap_period,
        })
        .collect()
}
