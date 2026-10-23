use std::fmt;

use cucumber::World;
use lb_testing_framework::NodeHttpClient;
use logos_blockchain_tests::cucumber::{
    error::{StepError, StepResult},
    runner::ScenarioCleanup,
    steps::nodes::stop_all_nodes,
    world::CucumberWorld,
};
use reqwest::Url;

use super::zone::ZoneState;
pub use super::zone::{
    ZoneDiscardedPayloads, ZoneReaderConfig, ZoneSequencerStartup, ZoneViewViolation,
};

#[derive(Default, World)]
pub struct ZoneWorld {
    pub node: CucumberWorld,
    pub zone: ZoneState,
}

impl AsMut<CucumberWorld> for ZoneWorld {
    fn as_mut(&mut self) -> &mut CucumberWorld {
        &mut self.node
    }
}

impl ScenarioCleanup for ZoneWorld {
    async fn cleanup(&mut self) -> StepResult {
        self.zone.clear();

        // Setup can fail before a cluster exists.
        if self.node.cluster.local_app.is_none() && self.node.cluster.k8s_manual_cluster.is_none() {
            return self.node.cleanup().await;
        }

        stop_all_nodes(&mut self.node).await
    }
}

impl Drop for ZoneWorld {
    fn drop(&mut self) {
        // Stop clients before the node world releases their cluster and files.
        self.zone.clear();
    }
}

impl fmt::Debug for ZoneWorld {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ZoneWorld")
            .field("node", &self.node)
            .field("zone", &self.zone.debug_summary())
            .finish()
    }
}

impl ZoneWorld {
    pub fn full_debug_info_string(&self) -> String {
        format!(
            "{}\n\nZoneSDK state:\n{}",
            self.node.full_debug_info_string(),
            self.zone.debug_summary(),
        )
    }

    pub fn zone_node_http_client(&self) -> Result<NodeHttpClient, StepError> {
        self.node.resolve_node_http_client(self.zone.node_name()?)
    }

    pub fn zone_node_url(&self) -> Result<Url, StepError> {
        Ok(self.zone_node_http_client()?.base_url().clone())
    }

    pub fn zone_node_http_client_for_sequencer(
        &self,
        alias: &str,
    ) -> Result<NodeHttpClient, StepError> {
        self.node
            .resolve_node_http_client(self.zone.sequencer_node_name(alias)?)
    }

    pub fn zone_node_url_for_sequencer(&self, alias: &str) -> Result<Url, StepError> {
        Ok(self
            .zone_node_http_client_for_sequencer(alias)?
            .base_url()
            .clone())
    }
}
