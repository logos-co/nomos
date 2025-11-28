use nomos_blend_service::core::network::libp2p::Libp2pBroadcastSettings;
use nomos_libp2p::PeerId;

use crate::config::cryptarchia::{deployment::Settings as DeploymentSettings, serde::Config};

pub mod deployment;
pub mod serde;

pub struct ServiceConfig {
    pub user: Config,
    pub deployment: DeploymentSettings,
}

impl From<ServiceConfig>
    for (
        chain_service::CryptarchiaSettings,
        chain_network::ChainNetworkSettings<
            PeerId,
            chain_network::network::adapters::libp2p::LibP2pAdapterSettings,
        >,
        chain_leader::LeaderSettings<(), Libp2pBroadcastSettings>,
    )
{
    fn from(value: ServiceConfig) -> Self {
        let chain_service_settings = chain_service::CryptarchiaSettings {
            bootstrap: value.user.service.bootstrap,
            config: value.deployment.ledger.clone(),
            recovery_file: value.user.service.recovery_file,
            starting_state: value.user.service.starting_state,
        };
        let chain_network_settings = chain_network::ChainNetworkSettings {
            bootstrap: value.user.network.bootstrap,
            config: value.deployment.ledger.clone(),
            network_adapter_settings: value.user.network.adapter,
            sync: value.user.network.sync,
        };
        let chain_leader_settings = chain_leader::LeaderSettings {
            blend_broadcast_settings: value.deployment.leader.broadcast,
            config: value.deployment.ledger,
            leader_config: value.user.leader.leader,
            transaction_selector_settings: (),
        };
        (
            chain_service_settings,
            chain_network_settings,
            chain_leader_settings,
        )
    }
}
