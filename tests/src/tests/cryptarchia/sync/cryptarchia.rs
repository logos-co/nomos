use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    hash::Hash,
};

use async_trait::async_trait;
use cryptarchia_consensus::network::adapters::libp2p::{LibP2pAdapter, LibP2pAdapterSettings};
use cryptarchia_consensus::network::{NetworkAdapter, SyncRequest};
use cryptarchia_consensus::storage::sync::SyncBlocksProvider;
use cryptarchia_engine::Slot;
use cryptarchia_sync::adapter::{CryptarchiaAdapter, CryptarchiaAdapterError};
use cryptarchia_sync_network::behaviour::BehaviourSyncReply;
use cryptarchia_sync_network::SyncRequestKind;
use futures_util::StreamExt;
use nomos_core::block::AbstractBlock;
use nomos_core::header::HeaderId;
use nomos_core::wire;
use nomos_network::NetworkService;
use nomos_node::{NetworkBackend, Wire};
use nomos_storage::backends::rocksdb::RocksBackend;
use nomos_storage::StorageMsg;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestBlock {
    pub id: HeaderId,
    pub parent: Option<HeaderId>,
    pub slot: Slot,
    pub data: Vec<u8>,
}

impl AbstractBlock for TestBlock {
    type Id = HeaderId;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn parent(&self) -> Self::Id {
        self.parent.unwrap_or_else(|| [0; 32].into())
    }

    fn slot(&self) -> Slot {
        self.slot
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct CryptarchiaSyncServiceConfig {
    pub topic: String,
}

pub struct CryptarchiaSyncService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkBackend, RuntimeServiceId>>
        + AsServiceId<nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>>
        + Clone
        + Debug
        + Display
        + Send
        + Sync
        + 'static,
{
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
    blocks: HashMap<HeaderId, TestBlock>,
    tip: Slot,
}

impl<RuntimeServiceId> ServiceData for CryptarchiaSyncService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkBackend, RuntimeServiceId>>
        + AsServiceId<nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>>
        + Clone
        + Debug
        + Display
        + Send
        + Sync
        + 'static,
{
    type Settings = CryptarchiaSyncServiceConfig;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ();
}

#[async_trait]
impl<RuntimeServiceId> ServiceCore<RuntimeServiceId> for CryptarchiaSyncService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkBackend, RuntimeServiceId>>
        + AsServiceId<nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>>
        + Clone
        + Debug
        + Display
        + Send
        + Sync
        + 'static,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_state,
            blocks: HashMap::new(),
            tip: Slot::genesis(),
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
        let network_adapter: LibP2pAdapter<TestBlock, RuntimeServiceId> =
            LibP2pAdapter::new(libp2p_settings, network_relay).await;

        let mut incoming_sync_requests = network_adapter.sync_requests_stream().await?;
        let storage_relay = self
            .service_state
            .overwatch_handle
            .relay::<nomos_storage::StorageService<_, _>>()
            .await
            .expect("Relay connection with StorageService should succeed");

        self.insert_test_blocks(100).await?;

        let sync_data_provider: SyncBlocksProvider<
            RocksBackend<Wire>,
            TestBlock,
            RuntimeServiceId,
        > = SyncBlocksProvider::new(
            storage_relay,
            self.service_state.overwatch_handle.runtime().clone(),
        );

        self.service_state
            .overwatch_handle
            .runtime()
            .spawn(async move {
                while let Some(sync_request) = incoming_sync_requests.next().await {
                    let SyncRequest {
                        kind,
                        reply_channel,
                    } = &sync_request;
                    match kind {
                        SyncRequestKind::Tip => {
                            reply_channel
                                .send(BehaviourSyncReply::TipData(100))
                                .await
                                .expect("Failed to send tip response");
                        }
                        _ => {
                            info!("Received sync request {:?}", kind);
                            sync_data_provider.process_sync_request(sync_request);
                        }
                    }
                }
            });

        cryptarchia_sync::Synchronization::run(self, &network_adapter).await?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl<RuntimeServiceId> CryptarchiaAdapter for CryptarchiaSyncService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkBackend, RuntimeServiceId>>
        + AsServiceId<nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>>
        + Clone
        + Debug
        + Display
        + Send
        + Sync
        + 'static,
{
    type Block = TestBlock;

    async fn process_block(&mut self, block: Self::Block) -> Result<(), CryptarchiaAdapterError> {
        info!("PROCESSING BLOCK {:?}", block.id());
        if self.has_block(&block.id()) {
            return Ok(());
        }
        self.tip = block.slot();
        info!("NEW TIP {:?}", self.tip);
        self.blocks.insert(block.id(), block);
        Ok(())
    }

    fn tip_slot(&self) -> Slot {
        self.tip
    }

    fn has_block(&self, id: &<Self::Block as AbstractBlock>::Id) -> bool {
        self.blocks.contains_key(id)
    }
}

impl<RuntimeServiceId> CryptarchiaSyncService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkBackend, RuntimeServiceId>>
        + AsServiceId<nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>>
        + Clone
        + Debug
        + Display
        + Send
        + Sync
        + 'static,
{
    pub async fn insert_test_blocks(&self, count: u64) -> Result<(), DynError> {
        let storage_relay = self
            .service_state
            .overwatch_handle
            .relay::<nomos_storage::StorageService<_, _>>()
            .await
            .expect("Relay connection with StorageService should succeed");

        for i in 0..count {
            let parent = if i == 0 {
                None
            } else {
                Some([i as u8 - 1; 32].into())
            };
            let block = TestBlock {
                id: [i as u8; 32].into(),
                parent,
                slot: Slot::from(i),
                data: vec![],
            };

            let key = format!("blocks_epoch_0_slot_{i}");
            let key = key.into_bytes();
            // let store_msg = nomos_storage::StorageMsg::new_store_message(key, block.clone());
            storage_relay
                .send(StorageMsg::Store {
                    key: key.into(),
                    value: wire::serialize(&block).unwrap().into(),
                })
                .await
                .unwrap();

            // if let Err((e, _)) = storage_relay.send(store_msg).await {
            //     tracing::error!("Failed to send storage message: {}", e);
            //     return Err(e.into());
            // }
        }

        Ok(())
    }
}
