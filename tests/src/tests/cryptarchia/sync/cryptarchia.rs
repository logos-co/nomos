use crate::id_from_u64;
use cryptarchia_consensus::network::adapters::libp2p::{LibP2pAdapter, LibP2pAdapterSettings};
use cryptarchia_consensus::network::NetworkAdapter;
use cryptarchia_consensus::storage::sync::SyncBlocksProvider;
use cryptarchia_engine::Slot;
use cryptarchia_sync::adapter::{CryptarchiaAdapter, CryptarchiaAdapterError};
use cryptarchia_sync::Synchronization;
use cryptarchia_sync_network::behaviour::BehaviourSyncReply;
use cryptarchia_sync_network::SyncRequestKind;
use futures_util::StreamExt;
use nomos_core::block::AbstractBlock;
use nomos_core::header::HeaderId;
use nomos_core::wire;
use nomos_libp2p::libp2p::bytes::Bytes;
use nomos_network::NetworkService;
use nomos_node::{NetworkBackend, Wire};
use nomos_storage::backends::rocksdb::RocksBackend;
use nomos_storage::{StorageMsg, StorageService};
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use serde::{Deserialize, Serialize};
use services_utils::overwatch::lifecycle;
use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    hash::Hash,
};
use tokio::sync::oneshot;
use tracing::info;

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestBlock {
    pub id: HeaderId,
    pub parent: Option<HeaderId>,
    pub slot: Slot,
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

impl From<(u64, Option<u64>, u64)> for TestBlock {
    fn from(args: (u64, Option<u64>, u64)) -> Self {
        let (id, parent, slot) = args;
        Self {
            id: id_from_u64(id),
            parent: parent.map(id_from_u64),
            slot: Slot::from(slot),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptarchiaSyncServiceConfig {
    pub topic: String,
    pub active: bool,
    pub initial_blocks: Vec<TestBlock>,
}

pub struct CryptarchiaSyncService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkBackend, RuntimeServiceId>>
        + AsServiceId<StorageService<RocksBackend<Wire>, RuntimeServiceId>>
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

#[derive(Debug)]
pub struct CryptarchiaSyncServiceMessage {
    pub(crate) reply_tx: oneshot::Sender<Vec<TestBlock>>,
}

impl<RuntimeServiceId> ServiceData for CryptarchiaSyncService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkBackend, RuntimeServiceId>>
        + AsServiceId<StorageService<RocksBackend<Wire>, RuntimeServiceId>>
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
    type Message = CryptarchiaSyncServiceMessage;
}

#[async_trait::async_trait]
impl<RuntimeServiceId> ServiceCore<RuntimeServiceId> for CryptarchiaSyncService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkBackend, RuntimeServiceId>>
        + AsServiceId<StorageService<RocksBackend<Wire>, RuntimeServiceId>>
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
        let config = self.service_state.settings_reader.get_updated_settings();
        self.insert_test_blocks(config.initial_blocks.clone()).await;

        let network_relay = self
            .service_state
            .overwatch_handle
            .relay::<NetworkService<NetworkBackend, RuntimeServiceId>>()
            .await
            .expect("Network relay should connect");

        let libp2p_settings = LibP2pAdapterSettings {
            topic: config.topic.clone(),
        };
        let network_adapter = LibP2pAdapter::new(libp2p_settings, network_relay).await;

        if config.active {
            self.run_active_mode(network_adapter).await?;
        } else {
            self.run_passive_mode(network_adapter).await?;
        }

        info!("Sync service finished: {}", config.active);
        Ok(())
    }
}

impl<RuntimeServiceId> CryptarchiaSyncService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkBackend, RuntimeServiceId>>
        + AsServiceId<StorageService<RocksBackend<Wire>, RuntimeServiceId>>
        + Clone
        + Debug
        + Display
        + Send
        + Sync
        + 'static,
{
    async fn run_active_mode(
        mut self,
        network_adapter: LibP2pAdapter<TestBlock, RuntimeServiceId>,
    ) -> Result<(), DynError> {
        let result_tx;
        if let Some(msg) = self.service_state.inbound_relay.next().await {
            result_tx = msg.reply_tx;
        } else {
            return Err("Failed to receive message".into());
        }

        let service = Synchronization::run(self, &network_adapter).await?;

        result_tx
            .send(service.blocks.clone().values().cloned().collect::<Vec<_>>())
            .expect("Failed to send blocks");

        service.service_state.overwatch_handle.shutdown().await;
        Ok(())
    }

    async fn run_passive_mode(
        self,
        network_adapter: LibP2pAdapter<TestBlock, RuntimeServiceId>,
    ) -> Result<(), DynError> {
        let storage_relay = self
            .service_state
            .overwatch_handle
            .relay::<StorageService<RocksBackend<Wire>, RuntimeServiceId>>()
            .await
            .expect("Storage relay should connect");

        let sync_data_provider: SyncBlocksProvider<
            RocksBackend<Wire>,
            TestBlock,
            RuntimeServiceId,
        > = SyncBlocksProvider::new(
            storage_relay,
            self.service_state.overwatch_handle.runtime().clone(),
        );

        let mut incoming_sync_requests = network_adapter.sync_requests_stream().await?;
        let tip = u64::from_be_bytes(self.tip.to_be_bytes());

        let mut lifecycle_stream = self.service_state.lifecycle_handle.message_stream();
        self.service_state
            .overwatch_handle
            .runtime()
            .spawn(async move {
                loop {
                    tokio::select! {
                        Some(sync_request) = incoming_sync_requests.next() => {
                            match sync_request.kind {
                                SyncRequestKind::Tip => {
                                    sync_request
                                        .reply_channel
                                        .send(BehaviourSyncReply::TipData(tip))
                                        .await
                                        .expect("Failed to send tip response");
                                }
                                _ => {
                                    sync_data_provider.process_sync_request(sync_request);
                                }
                            }
                        },
                        Some(msg) = lifecycle_stream.next() => {
                            if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                                break;
                            }
                         }
                    }
                }
            });

        Ok(())
    }

    async fn insert_test_blocks(&mut self, blocks: Vec<TestBlock>) {
        let storage_relay = self
            .service_state
            .overwatch_handle
            .relay::<StorageService<RocksBackend<Wire>, RuntimeServiceId>>()
            .await
            .expect("Storage relay should connect");

        for block in blocks {
            // Not sure if randomness in storage layer is fine for a blockchain if functionality is nevertheless deterministic
            // But probably better to avoid outside test code.
            let rnd_u8 = rand::random::<u16>();
            let slot = u64::from_be_bytes(block.slot.to_be_bytes());
            let key = format!("blocks_epoch_0_slot_{slot}_{rnd_u8}").into_bytes();
            storage_relay
                .send(StorageMsg::Store {
                    key: key.into(),
                    value: wire::serialize(&block).unwrap().into(),
                })
                .await
                .unwrap();

            let id_key: [u8; 32] = block.id.into();
            storage_relay
                .send(StorageMsg::Store {
                    key: Bytes::copy_from_slice(&id_key),
                    value: wire::serialize(&block).unwrap().into(),
                })
                .await
                .unwrap();

            self.tip = block.slot();
            self.blocks.insert(block.id(), block);
        }
    }
}

#[async_trait::async_trait]
impl<RuntimeServiceId> CryptarchiaAdapter for CryptarchiaSyncService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkBackend, RuntimeServiceId>>
        + AsServiceId<StorageService<RocksBackend<Wire>, RuntimeServiceId>>
        + Clone
        + Debug
        + Display
        + Send
        + Sync
        + 'static,
{
    type Block = TestBlock;

    async fn process_block(&mut self, block: Self::Block) -> Result<(), CryptarchiaAdapterError> {
        if self.has_block(&block.id()) {
            return Ok(());
        } else if !self.blocks.contains_key(&block.parent.unwrap()) {
            return Err(CryptarchiaAdapterError::ParentNotFound);
        }

        if self.tip < block.slot() {
            self.tip = block.slot();
        }
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
