use std::marker::PhantomData;

use cryptarchia_engine::{Boostrapping, Online};

use crate::{bootstrap::cryptarchia::InitialCryptarchia, network::NetworkAdapter, Cryptarchia};

pub struct InitialBlockDownload<NetAdapter, RuntimeServiceId>
where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
{
    _phantom: PhantomData<(NetAdapter, RuntimeServiceId)>,
}

impl<NetAdapter, RuntimeServiceId> InitialBlockDownload<NetAdapter, RuntimeServiceId>
where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
{
    pub async fn run(
        cryptarchia: InitialCryptarchia,
        network_adapter: NetAdapter,
    ) -> IbdCompletedCryptarchia {
        match cryptarchia {
            InitialCryptarchia::Bootstrapping(cryptarchia) => {
                let cryptarchia = Self::run_with(cryptarchia, network_adapter).await;
                IbdCompletedCryptarchia::Bootstrapping(cryptarchia)
            }
            InitialCryptarchia::Online(cryptarchia) => {
                let cryptarchia = Self::run_with(cryptarchia, network_adapter).await;
                IbdCompletedCryptarchia::Online(cryptarchia)
            }
        }
    }

    #[expect(clippy::unused_async, reason = "To be implemented")]
    async fn run_with<CryptarchiaState>(
        cryptarchia: Cryptarchia<CryptarchiaState>,
        _network_adapter: NetAdapter,
    ) -> Cryptarchia<CryptarchiaState>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState,
    {
        // TODO: Implement IBD
        cryptarchia
    }
}

pub enum IbdCompletedCryptarchia {
    Bootstrapping(Cryptarchia<Boostrapping>),
    Online(Cryptarchia<Online>),
}
