use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
};

use async_trait::async_trait;
use cryptarchia_consensus::network::adapters::libp2p::{LibP2pAdapter, LibP2pAdapterSettings};
use cryptarchia_consensus::network::NetworkAdapter as ConsensusNetworkAdapter;
use cryptarchia_engine::Slot;
use cryptarchia_sync::adapter::{CryptarchiaAdapter, CryptarchiaAdapterError, NetworkAdapter};
use nomos_core::block::AbstractBlock;
use nomos_core::header::HeaderId;
use nomos_network::NetworkService;
use nomos_storage::backends::StorageBackend;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MockBlock {
    id: HeaderId,
    slot: Slot,
}

impl AbstractBlock for MockBlock {
    type Id = HeaderId;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn parent(&self) -> Self::Id {
        self.id
    }

    fn slot(&self) -> Slot {
        self.slot
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct CryptarchiaSyncServiceConfig {
    pub topic: String,
}

pub struct CryptarchiaSyncService<Storage, Network, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync + 'static,
    Network: ConsensusNetworkAdapter<RuntimeServiceId> + Sync + Send + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + 'static,
{
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
    blocks: HashMap<HeaderId, MockBlock>,
    tip_slot: Slot,
    _marker: PhantomData<(Storage, Network)>,
}

impl<Storage, Network, RuntimeServiceId> ServiceData
    for CryptarchiaSyncService<Storage, Network, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync + 'static,
    Network: ConsensusNetworkAdapter<RuntimeServiceId> + Sync + Send + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + 'static,
{
    type Settings = CryptarchiaSyncServiceConfig;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ();
}

#[async_trait]
impl<Storage, Network, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for CryptarchiaSyncService<Storage, Network, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync + 'static,
    Network: ConsensusNetworkAdapter<RuntimeServiceId> + Sync + Send + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + 'static,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_state,
            blocks: HashMap::new(),
            tip_slot: Slot::genesis(),
            _marker: PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
        let CryptarchiaSyncServiceConfig { topic } =
            self.service_state.settings_reader.get_updated_settings();

        let network_relay = self
            .service_state
            .overwatch_handle
            .relay::<NetworkService<_, _>>()
            .await
            .expect("Relay connection with NetworkService should succeed");

        let libp2p_settings = LibP2pAdapterSettings {
            topic: topic.clone(),
        };

        let network_relay = network_relay.clone();
        let network_adapter = LibP2pAdapter::new(libp2p_settings, network_relay).await?;

        let sync_adapter = cryptarchia_sync::Synchronization::run(self, &network_adapter).await?;
        // Service loop
        // loop {
        // tokio::select! {
        //     _ = self.service_state.recv() => {
        //         // Handle service messages
        //     }
        // }
        // }
        Ok(())
    }
}

#[async_trait::async_trait]
impl<Storage, Network, RuntimeServiceId> CryptarchiaAdapter
    for CryptarchiaSyncService<Storage, Network, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + 'static,
{
    type Block = MockBlock;

    async fn process_block(&mut self, block: Self::Block) -> Result<(), CryptarchiaAdapterError> {
        todo!();
    }

    fn tip_slot(&self) -> Slot {
        self.tip_slot
    }

    fn has_block(&self, id: &<Self::Block as AbstractBlock>::Id) -> bool {
        self.blocks.contains_key(id)
    }
}
