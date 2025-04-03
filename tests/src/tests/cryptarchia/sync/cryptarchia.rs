use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
    pin::Pin,
};

use async_trait::async_trait;
use cryptarchia_engine::Slot;
use cryptarchia_sync::adapter::{CryptarchiaAdapter, CryptarchiaAdapterError, NetworkAdapter};
use futures::Stream;
use nomos_core::block::AbstractBlock;
use nomos_core::header::HeaderId;
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
pub struct CryptarchiaSyncServiceConfig {}

pub struct CryptarchiaSyncService<Storage, Block, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync + 'static,
    Block: AbstractBlock + Clone + Eq + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    <Block as AbstractBlock>::Id: Eq + Hash + Clone + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + 'static,
{
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
    blocks: HashMap<<Block as AbstractBlock>::Id, Block>,
    tip_slot: Slot,
    _marker: PhantomData<(Storage, Block)>,
}

impl<Storage, Block, RuntimeServiceId> ServiceData
    for CryptarchiaSyncService<Storage, Block, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync + 'static,
    Block: AbstractBlock + Clone + Eq + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    <Block as AbstractBlock>::Id: Eq + Hash + Clone + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + 'static,
{
    type Settings = CryptarchiaSyncServiceConfig;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ();
}

#[async_trait]
impl<Storage, Block, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for CryptarchiaSyncService<Storage, Block, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync + 'static,
    Block: AbstractBlock + Clone + Eq + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    <Block as AbstractBlock>::Id: Eq + Hash + Clone + Send + Sync + 'static,
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
impl<Storage, Block, RuntimeServiceId> CryptarchiaAdapter
    for CryptarchiaSyncService<Storage, Block, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync + 'static,
    Block: AbstractBlock + Clone + Eq + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    <Block as AbstractBlock>::Id: Eq + Hash + Clone + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + 'static,
{
    type Block = Block;

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
