use std::marker::PhantomData;

use crate::{network::NetworkAdapter, Cryptarchia};

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
    #[expect(clippy::unused_async, reason = "To be implemented")]
    pub async fn run(cryptarchia: Cryptarchia, _network_adapter: NetAdapter) -> Cryptarchia {
        // TODO: Implement IBD
        cryptarchia
    }
}
