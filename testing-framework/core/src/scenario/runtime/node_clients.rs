use rand::{Rng as _, thread_rng};

use crate::{
    nodes::ApiClient,
    topology::{GeneratedTopology, Topology},
};

#[derive(Clone, Default)]
pub struct NodeClients {
    validators: Vec<ApiClient>,
    executors: Vec<ApiClient>,
}

impl NodeClients {
    #[must_use]
    pub const fn new(validators: Vec<ApiClient>, executors: Vec<ApiClient>) -> Self {
        Self {
            validators,
            executors,
        }
    }

    #[must_use]
    pub fn from_topology(_descriptors: &GeneratedTopology, topology: &Topology) -> Self {
        let validator_clients = topology.validators().iter().map(|node| {
            let testing = node.testing_url();
            ApiClient::from_urls(node.url(), testing)
        });

        let executor_clients = topology.executors().iter().map(|node| {
            let testing = node.testing_url();
            ApiClient::from_urls(node.url(), testing)
        });

        Self::new(validator_clients.collect(), executor_clients.collect())
    }

    #[must_use]
    pub fn validator_clients(&self) -> &[ApiClient] {
        &self.validators
    }

    #[must_use]
    pub fn executor_clients(&self) -> &[ApiClient] {
        &self.executors
    }

    #[must_use]
    pub fn random_validator(&self) -> Option<&ApiClient> {
        if self.validators.is_empty() {
            return None;
        }
        let mut rng = thread_rng();
        let idx = rng.gen_range(0..self.validators.len());
        self.validators.get(idx)
    }

    #[must_use]
    pub fn random_executor(&self) -> Option<&ApiClient> {
        if self.executors.is_empty() {
            return None;
        }
        let mut rng = thread_rng();
        let idx = rng.gen_range(0..self.executors.len());
        self.executors.get(idx)
    }

    pub fn all_clients(&self) -> impl Iterator<Item = &ApiClient> {
        self.validators.iter().chain(self.executors.iter())
    }

    #[must_use]
    pub fn any_client(&self) -> Option<&ApiClient> {
        let validator_count = self.validators.len();
        let executor_count = self.executors.len();
        let total = validator_count + executor_count;
        if total == 0 {
            return None;
        }
        let mut rng = thread_rng();
        let choice = rng.gen_range(0..total);
        if choice < validator_count {
            self.validators.get(choice)
        } else {
            self.executors.get(choice - validator_count)
        }
    }
}
