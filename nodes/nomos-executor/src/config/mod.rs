use color_eyre::eyre::Result;
use nomos_blend_service::{
    backends::libp2p::Libp2pBlendBackend as BlendBackend,
    network::libp2p::Libp2pAdapter as BlendNetworkAdapter, BlendService,
};
use nomos_da_network_service::{
    backends::libp2p::executor::DaNetworkExecutorBackend, NetworkService as DaNetworkService,
};
use nomos_network::backends::libp2p::Libp2p as NetworkBackend;
use nomos_node::{
    config::{
        mempool::MempoolConfig, update_blend, update_cryptarchia_consensus, update_network,
        BlendArgs,
    },
    CryptarchiaArgs, HttpArgs, LogArgs, NetworkArgs, NetworkService, NomosTimeService, Wire,
};
use nomos_storage::backends::rocksdb::RocksBackend;
use overwatch::services::ServiceData;
use serde::{Deserialize, Serialize};
use subnetworks_assignations::versions::v1::FillFromNodeList;

use crate::{ExecutorApiService, RuntimeServiceId};

#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct Config {
    #[cfg(feature = "tracing")]
    pub tracing: <nomos_node::Tracing<RuntimeServiceId> as ServiceData>::Settings,
    pub network: <NetworkService<NetworkBackend, nomos_node::RuntimeServiceId> as ServiceData>::Settings,
    pub blend: <BlendService<BlendBackend, BlendNetworkAdapter<RuntimeServiceId>, RuntimeServiceId> as ServiceData>::Settings,
    pub da_dispersal: <crate::DaDispersal as ServiceData>::Settings,
    pub da_network:
        <DaNetworkService<DaNetworkExecutorBackend<FillFromNodeList>, RuntimeServiceId> as ServiceData>::Settings,
    pub da_indexer: <crate::ExecutorDaIndexer as ServiceData>::Settings,
    pub da_verifier: <crate::ExecutorDaVerifier as ServiceData>::Settings,
    pub da_sampling: <crate::ExecutorDaSampling as ServiceData>::Settings,
    pub http: <ExecutorApiService as ServiceData>::Settings,
    pub cryptarchia: <crate::ExecutorCryptarchia as ServiceData>::Settings,
    pub time: <NomosTimeService<RuntimeServiceId> as ServiceData>::Settings,
    pub storage: <crate::StorageService<RocksBackend<Wire>, RuntimeServiceId> as ServiceData>::Settings,
    pub mempool: MempoolConfig,
}

impl Config {
    pub fn update_from_args(
        mut self,
        #[cfg_attr(
            not(feature = "tracing"),
            expect(
                unused_variables,
                reason = "`log_args` is only used to update tracing configs when the `tracing` feature is enabled."
            )
        )]
        log_args: LogArgs,
        network_args: NetworkArgs,
        blend_args: BlendArgs,
        http_args: HttpArgs,
        cryptarchia_args: CryptarchiaArgs,
    ) -> Result<Self> {
        #[cfg(feature = "tracing")]
        nomos_node::config::update_tracing(&mut self.tracing, log_args)?;
        update_network(&mut self.network, network_args)?;
        update_blend(&mut self.blend, blend_args)?;
        update_http(&mut self.http, http_args)?;
        update_cryptarchia_consensus(&mut self.cryptarchia, cryptarchia_args)?;
        Ok(self)
    }
}

pub fn update_http(
    http: &mut <ExecutorApiService as ServiceData>::Settings,
    http_args: HttpArgs,
) -> Result<()> {
    let HttpArgs {
        http_addr,
        cors_origins,
    } = http_args;

    if let Some(addr) = http_addr {
        http.backend_settings.address = addr;
    }

    if let Some(cors) = cors_origins {
        http.backend_settings.cors_origins = cors;
    }

    Ok(())
}
