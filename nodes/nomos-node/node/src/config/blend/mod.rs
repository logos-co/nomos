use overwatch::services::ServiceData;

use crate::{
    BlendCoreService, BlendEdgeService, BlendService,
    config::{blend::serde::Config, deployment::Settings as DeploymentSettings},
};

pub mod deployment;
pub mod serde;

pub struct ServiceConfig {
    serde_config: Config,
    deployment_config: DeploymentSettings,
}

impl ServiceConfig {
    #[must_use]
    pub const fn from_config_and_deployment(
        serde_config: Config,
        deployment_config: DeploymentSettings,
    ) -> Self {
        Self {
            serde_config,
            deployment_config,
        }
    }
}

impl From<ServiceConfig>
    for (
        <BlendService as ServiceData>::Settings,
        <BlendCoreService as ServiceData>::Settings,
        <BlendEdgeService as ServiceData>::Settings,
    )
{
    fn from(_config: ServiceConfig) -> Self {
        unimplemented!()
    }
}
