use color_eyre::eyre::Result;
use nomos_node::{
    config::{
        mempool::MempoolConfig, update_blend, update_cryptarchia_consensus, update_network,
        BlendArgs,
    },
    CryptarchiaArgs, HttpArgs, LogArgs, NetworkArgs,
};
use overwatch::services::ServiceData;
use serde::{Deserialize, Serialize};

use crate::{
    ApiService, BlendService, CryptarchiaService, DaDispersalService, DaIndexerService,
    DaNetworkService, DaSamplingService, DaVerifierService, NetworkService, RuntimeServiceId,
    StorageService, TimeService,
};

#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct Config {
    #[cfg(feature = "tracing")]
    pub tracing: <nomos_node::Tracing<RuntimeServiceId> as ServiceData>::Settings,
    pub network: <NetworkService as ServiceData>::Settings,
    pub blend: <BlendService as ServiceData>::Settings,
    pub da_dispersal: <DaDispersalService as ServiceData>::Settings,
    pub da_network: <DaNetworkService as ServiceData>::Settings,
    pub da_indexer: <DaIndexerService as ServiceData>::Settings,
    pub da_verifier: <DaVerifierService as ServiceData>::Settings,
    pub da_sampling: <DaSamplingService as ServiceData>::Settings,
    pub http: <ApiService as ServiceData>::Settings,
    pub cryptarchia: <CryptarchiaService as ServiceData>::Settings,
    pub time: <TimeService as ServiceData>::Settings,
    pub storage: <StorageService as ServiceData>::Settings,
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
        update_network::<RuntimeServiceId>(&mut self.network, network_args)?;
        update_blend(&mut self.blend, blend_args)?;
        update_http(&mut self.http, http_args)?;
        update_cryptarchia_consensus(&mut self.cryptarchia, cryptarchia_args)?;
        Ok(self)
    }
}

pub fn update_http(
    http: &mut <ApiService as ServiceData>::Settings,
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
