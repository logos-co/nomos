use crate::config::cryptarchia::{deployment::Settings as DeploymentSettings, serde::Config};

pub mod deployment;
pub mod serde;

pub struct ServiceConfig {
    pub user: Config,
    pub deployment: DeploymentSettings,
}

impl From<ServiceConfig> for chain_service::CryptarchiaSettings {
    fn from(value: ServiceConfig) -> Self {
        Self {
            bootstrap: value.user.bootstrap,
            config: value.deployment.ledger,
            recovery_file: value.user.recovery_file,
            starting_state: value.user.starting_state,
        }
    }
}
