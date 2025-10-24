use color_eyre::eyre::Result;
use nomos_node::{
    CryptarchiaLeaderArgs, HttpArgs, LogArgs, NetworkArgs,
    config::{
        BlendArgs, blend::BlendConfig, mempool::MempoolConfig, update_blend,
        update_cryptarchia_leader_consensus, update_network,
    },
    generic_services::{MembershipService, SdpService},
};
use overwatch::services::ServiceData;
use serde::{Deserialize, Serialize};

use crate::{
    ApiService, CryptarchiaLeaderService, CryptarchiaService, DaDispersalService, DaNetworkService,
    DaSamplingService, DaVerifierService, NetworkService, RuntimeServiceId, StorageService,
    TimeService, WalletService,
};

#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct Config {
    #[cfg(feature = "tracing")]
    pub tracing: <nomos_node::Tracing<RuntimeServiceId> as ServiceData>::Settings,
    pub network: <NetworkService as ServiceData>::Settings,
    pub blend: BlendConfig,
    pub da_dispersal: <DaDispersalService as ServiceData>::Settings,
    pub da_network: <DaNetworkService as ServiceData>::Settings,
    pub membership: <MembershipService<RuntimeServiceId> as ServiceData>::Settings,
    pub sdp: <SdpService<RuntimeServiceId> as ServiceData>::Settings,
    pub da_verifier: <DaVerifierService as ServiceData>::Settings,
    pub da_sampling: <DaSamplingService as ServiceData>::Settings,
    pub http: <ApiService as ServiceData>::Settings,
    pub cryptarchia: <CryptarchiaService as ServiceData>::Settings,
    pub cryptarchia_leader: <CryptarchiaLeaderService as ServiceData>::Settings,
    pub time: <TimeService as ServiceData>::Settings,
    pub storage: <StorageService as ServiceData>::Settings,
    pub mempool: MempoolConfig,
    pub wallet: <WalletService as ServiceData>::Settings,

    #[cfg(feature = "testing")]
    pub testing_http: <ApiService as ServiceData>::Settings,
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
        cryptarchia_leader_args: CryptarchiaLeaderArgs,
    ) -> Result<Self> {
        #[cfg(feature = "tracing")]
        nomos_node::config::update_tracing(&mut self.tracing, log_args)?;
        update_network::<RuntimeServiceId>(&mut self.network, network_args)?;
        update_blend(&mut self.blend, blend_args)?;
        update_http(&mut self.http, http_args)?;
        update_cryptarchia_leader_consensus(&mut self.cryptarchia_leader, cryptarchia_leader_args)?;
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
