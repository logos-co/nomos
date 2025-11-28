use std::sync::Arc;

use nomos_core::sdp::{MinStake, ServiceParameters, ServiceType};
use serde::{Deserialize, Serialize};

use crate::config::deployment::Settings as DeploymentSettings;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    pub ledger: nomos_ledger::Config,
}

#[expect(clippy::fallible_impl_from, reason = "Well-known values.")]
impl From<DeploymentSettings> for Settings {
    fn from(value: DeploymentSettings) -> Self {
        match value {
            DeploymentSettings::Mainnet => Self {
                ledger: nomos_ledger::Config {
                    consensus_config: cryptarchia_engine::Config {
                        active_slot_coeff: 0.9,
                        security_param: 10.try_into().unwrap(),
                    },
                    epoch_config: cryptarchia_engine::EpochConfig {
                        epoch_period_nonce_buffer: 3.try_into().unwrap(),
                        epoch_period_nonce_stabilization: 4.try_into().unwrap(),
                        epoch_stake_distribution_stabilization: 3.try_into().unwrap(),
                    },
                    sdp_config: nomos_ledger::mantle::sdp::Config {
                        min_stake: MinStake {
                            threshold: 1,
                            timestamp: 0,
                        },
                        service_params: Arc::new(
                            [
                                (
                                    ServiceType::BlendNetwork,
                                    ServiceParameters {
                                        inactivity_period: 20,
                                        lock_period: 10,
                                        retention_period: 100,
                                        session_duration: 1_000,
                                        timestamp: 0,
                                    },
                                ),
                                (
                                    ServiceType::DataAvailability,
                                    ServiceParameters {
                                        inactivity_period: 20,
                                        lock_period: 10,
                                        retention_period: 100,
                                        session_duration: 1_000,
                                        timestamp: 0,
                                    },
                                ),
                            ]
                            .into_iter()
                            .collect(),
                        ),
                    },
                },
            },
            DeploymentSettings::Custom(custom_deployment_settings) => {
                custom_deployment_settings.cryptarchia
            }
        }
    }
}
