/// Re-export for `OpenAPI`
#[cfg(feature = "openapi")]
pub mod openapi {
    pub use crate::backend::Status;
}

use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
};

use futures::StreamExt;
use nomos_network::{NetworkMsg, NetworkService};
use overwatch::{
    services::{
        handle::ServiceStateHandle, relay::OutboundRelay, AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceStateHandle,
};
use services_utils::overwatch::{
    lifecycle, recovery::operators::RecoveryBackend as RecoveryBackendTrait, JsonFileBackend,
    RecoveryOperator,
};

use crate::{
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as NetworkAdapterTrait,
    tx::{settings::TxMempoolSettings, state::TxMempoolState},
    MempoolMetrics, MempoolMsg,
};

/// A tx mempool service that uses a [`JsonFileBackend`] as a recovery
/// mechanism.
pub type TxMempoolService<NetworkAdapter, Pool, RuntimeServiceId> = GenericTxMempoolService<
    Pool,
    NetworkAdapter,
    JsonFileBackend<
        TxMempoolState<
            <Pool as RecoverableMempool>::RecoveryState,
            <Pool as MemPool>::Settings,
            <NetworkAdapter as NetworkAdapterTrait<RuntimeServiceId>>::Settings,
        >,
        TxMempoolSettings<
            <Pool as MemPool>::Settings,
            <NetworkAdapter as NetworkAdapterTrait<RuntimeServiceId>>::Settings,
        >,
    >,
    RuntimeServiceId,
>;

/// A generic tx mempool service which wraps around a mempool, a network
/// adapter, and a recovery backend.
pub struct GenericTxMempoolService<Pool, NetworkAdapter, RecoveryBackend, RuntimeServiceId>
where
    Pool: RecoverableMempool,
    Pool::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId>,
    NetworkAdapter::Settings: Clone,
    RecoveryBackend: RecoveryBackendTrait,
{
    pool: Pool,
    service_state_handle: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(NetworkAdapter, RecoveryBackend)>,
}

impl<Pool, NetworkAdapter, RecoveryBackend, RuntimeServiceId>
    GenericTxMempoolService<Pool, NetworkAdapter, RecoveryBackend, RuntimeServiceId>
where
    Pool: RecoverableMempool,
    Pool::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId>,
    NetworkAdapter::Settings: Clone,
    RecoveryBackend: RecoveryBackendTrait,
{
    pub const fn new(
        pool: Pool,
        service_state_handle: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
    ) -> Self {
        Self {
            pool,
            service_state_handle,
            _phantom: PhantomData,
        }
    }
}

impl<Pool, NetworkAdapter, RecoveryBackend, RuntimeServiceId> ServiceData
    for GenericTxMempoolService<Pool, NetworkAdapter, RecoveryBackend, RuntimeServiceId>
where
    Pool: RecoverableMempool,
    Pool::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId>,
    NetworkAdapter::Settings: Clone,
    RecoveryBackend: RecoveryBackendTrait,
{
    type Settings = TxMempoolSettings<Pool::Settings, NetworkAdapter::Settings>;
    type State = TxMempoolState<Pool::RecoveryState, Pool::Settings, NetworkAdapter::Settings>;
    type StateOperator = RecoveryOperator<RecoveryBackend>;
    type Message = MempoolMsg<Pool::BlockId, Pool::Item, Pool::Item, Pool::Key>;
}

#[async_trait::async_trait]
impl<Pool, NetworkAdapter, RecoveryBackend, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for GenericTxMempoolService<Pool, NetworkAdapter, RecoveryBackend, RuntimeServiceId>
where
    Pool: RecoverableMempool + Send,
    Pool::RecoveryState: Debug + Send + Sync,
    Pool::Key: Send,
    Pool::Item: Clone + Send + 'static,
    Pool::BlockId: Send,
    Pool::Settings: Clone + Sync + Send,
    NetworkAdapter:
        NetworkAdapterTrait<RuntimeServiceId, Payload = Pool::Item, Key = Pool::Key> + Send,
    NetworkAdapter::Settings: Clone + Send + Sync + 'static,
    RecoveryBackend: RecoveryBackendTrait + Send,
    RuntimeServiceId: Display
        + Debug
        + Sync
        + Send
        + 'static
        + AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkAdapter::Backend, RuntimeServiceId>>,
{
    fn init(
        service_state: ServiceStateHandle<
            Self::Message,
            Self::Settings,
            Self::State,
            RuntimeServiceId,
        >,
        init_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        tracing::trace!(
            "Initializing TxMempoolService with initial state {:#?}",
            init_state.pool
        );
        let settings = service_state.settings_reader.get_updated_settings();
        let recovered_pool = init_state.pool.map_or_else(
            || Pool::new(settings.pool.clone()),
            |recovered_pool| Pool::recover(settings.pool.clone(), recovered_pool),
        );

        Ok(Self::new(recovered_pool, service_state))
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let network_service_relay = self
            .service_state_handle
            .overwatch_handle
            .relay::<NetworkService<_, _>>()
            .await
            .expect("Relay connection with NetworkService should succeed");

        // Queue for network messages
        let mut network_items = NetworkAdapter::new(
            self.service_state_handle
                .settings_reader
                .get_updated_settings()
                .network_adapter,
            network_service_relay.clone(),
        )
        .await
        .payload_stream()
        .await;
        // Queue for lifecycle messages
        let mut lifecycle_stream = self.service_state_handle.lifecycle_handle.message_stream();

        loop {
            tokio::select! {
                // Queue for relay messages
                Some(relay_msg) = self.service_state_handle.inbound_relay.recv() => {
                    self.handle_mempool_message(relay_msg, network_service_relay.clone());
                }
                Some((key, item )) = network_items.next() => {
                    self.pool.add_item(key, item).unwrap_or_else(|e| {
                        tracing::debug!("could not add item to the pool due to: {e}");
                    });
                    tracing::info!(counter.tx_mempool_pending_items = self.pool.pending_item_count());
                    self.service_state_handle.state_updater.update(self.pool.save().into());
                }
                Some(lifecycle_msg) = lifecycle_stream.next() =>  {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&lifecycle_msg) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

impl<Pool, NetworkAdapter, RecoveryBackend, RuntimeServiceId>
    GenericTxMempoolService<Pool, NetworkAdapter, RecoveryBackend, RuntimeServiceId>
where
    Pool: RecoverableMempool,
    Pool::Item: Clone + Send + 'static,
    Pool::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId, Payload = Pool::Item> + Send,
    NetworkAdapter::Settings: Clone + Send + 'static,
    RecoveryBackend: RecoveryBackendTrait,
    RuntimeServiceId: 'static,
{
    #[expect(
        clippy::cognitive_complexity,
        reason = "Mempool message handling is convenient to have in one block"
    )]
    fn handle_mempool_message(
        &mut self,
        message: MempoolMsg<Pool::BlockId, Pool::Item, Pool::Item, Pool::Key>,
        network_relay: OutboundRelay<NetworkMsg<NetworkAdapter::Backend, RuntimeServiceId>>,
    ) {
        match message {
            MempoolMsg::Add {
                payload: item,
                key,
                reply_channel,
            } => {
                match self.pool.add_item(key, item.clone()) {
                    Ok(_id) => {
                        // Broadcast the item to the network
                        let settings = self
                            .service_state_handle
                            .settings_reader
                            .get_updated_settings()
                            .network_adapter;
                        self.service_state_handle
                            .state_updater
                            .update(self.pool.save().into());
                        // move sending to a new task so local operations can complete in the
                        // meantime
                        tokio::spawn(async {
                            let adapter = NetworkAdapter::new(settings, network_relay).await;
                            adapter.send(item).await;
                        });
                        if let Err(e) = reply_channel.send(Ok(())) {
                            tracing::debug!("Failed to send reply to AddTx: {e:?}");
                        }
                    }
                    Err(e) => {
                        tracing::debug!("could not add tx to the pool due to: {e}");
                        if let Err(e) = reply_channel.send(Err(e)) {
                            tracing::debug!("Failed to send reply to AddTx: {e:?}");
                        }
                    }
                }
            }
            MempoolMsg::View {
                ancestor_hint,
                reply_channel,
            } => {
                reply_channel
                    .send(self.pool.view(ancestor_hint))
                    .unwrap_or_else(|_| tracing::debug!("could not send back pool view"));
            }
            MempoolMsg::MarkInBlock { ids, block } => {
                self.pool.mark_in_block(&ids, block);
            }
            #[cfg(test)]
            MempoolMsg::BlockItems {
                block,
                reply_channel,
            } => {
                reply_channel
                    .send(self.pool.block_items(block))
                    .unwrap_or_else(|_| tracing::debug!("could not send back block items"));
            }
            MempoolMsg::Prune { ids } => {
                self.pool.prune(&ids);
            }
            MempoolMsg::Metrics { reply_channel } => {
                let metrics = MempoolMetrics {
                    pending_items: self.pool.pending_item_count(),
                    last_item_timestamp: self.pool.last_item_timestamp(),
                };
                reply_channel
                    .send(metrics)
                    .unwrap_or_else(|_| tracing::debug!("could not send back mempool metrics"));
            }
            MempoolMsg::Status {
                items,
                reply_channel,
            } => {
                reply_channel
                    .send(self.pool.status(&items))
                    .unwrap_or_else(|_| tracing::debug!("could not send back mempool status"));
            }
        }
    }
}
