/// Re-export for `OpenAPI`
#[cfg(feature = "openapi")]
pub mod openapi {
    pub use crate::backend::Status;
}

use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use futures::StreamExt as _;
use nomos_da_sampling::backend::kzgrs::KzgrsSamplingBackend;
use nomos_network::{message::BackendNetworkMsg, NetworkService};
use overwatch::{
    services::{relay::OutboundRelay, AsServiceId, ServiceCore, ServiceData},
    OpaqueServiceResourcesHandle,
};
use services_utils::{
    overwatch::{
        recovery::operators::RecoveryBackend as RecoveryBackendTrait, JsonFileBackend,
        RecoveryOperator,
    },
    wait_until_services_are_ready,
};

use crate::{
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as NetworkAdapterTrait,
    processor::{tx::SignedTxProcessor, PayloadProcessor},
    tx::{settings::TxMempoolSettings, state::TxMempoolState},
    MempoolMetrics, MempoolMsg,
};

pub type DaSamplingService<SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId> =
    nomos_da_sampling::DaSamplingService<
        KzgrsSamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    >;

/// A tx mempool service that uses a [`JsonFileBackend`] as a recovery
/// mechanism.
pub type TxMempoolService<
    MempoolNetworkAdapter,
    SamplingNetworkAdapter,
    SamplingStorage,
    Pool,
    RuntimeServiceId,
> = GenericTxMempoolService<
    Pool,
    MempoolNetworkAdapter,
    SignedTxProcessor<DaSamplingService<SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>>,
    JsonFileBackend<
        TxMempoolState<
            <Pool as RecoverableMempool>::RecoveryState,
            <Pool as MemPool>::Settings,
            <MempoolNetworkAdapter as NetworkAdapterTrait<RuntimeServiceId>>::Settings,
            <SignedTxProcessor<
                DaSamplingService<SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>,
            > as PayloadProcessor>::Settings,
        >,
        TxMempoolSettings<
            <Pool as MemPool>::Settings,
            <MempoolNetworkAdapter as NetworkAdapterTrait<RuntimeServiceId>>::Settings,
            <SignedTxProcessor<
                DaSamplingService<SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>,
            > as PayloadProcessor>::Settings,
        >,
    >,
    RuntimeServiceId,
>;

/// A generic tx mempool service which wraps around a mempool, a network
/// adapter, and a recovery backend.
pub struct GenericTxMempoolService<
    Pool,
    NetworkAdapter,
    Processor,
    RecoveryBackend,
    RuntimeServiceId,
> where
    Pool: RecoverableMempool,
    Pool::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId>,
    NetworkAdapter::Settings: Clone,
    Processor: PayloadProcessor,
    Processor::Settings: Clone,
    RecoveryBackend: RecoveryBackendTrait,
{
    pool: Pool,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(NetworkAdapter, Processor, RecoveryBackend)>,
}

impl<Pool, NetworkAdapter, Processor, RecoveryBackend, RuntimeServiceId>
    GenericTxMempoolService<Pool, NetworkAdapter, Processor, RecoveryBackend, RuntimeServiceId>
where
    Pool: RecoverableMempool,
    Pool::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId>,
    NetworkAdapter::Settings: Clone,
    Processor: PayloadProcessor,
    Processor::Settings: Clone,
    RecoveryBackend: RecoveryBackendTrait,
{
    pub const fn new(
        pool: Pool,
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    ) -> Self {
        Self {
            pool,
            service_resources_handle,
            _phantom: PhantomData,
        }
    }
}

impl<Pool, NetworkAdapter, Processor, RecoveryBackend, RuntimeServiceId> ServiceData
    for GenericTxMempoolService<Pool, NetworkAdapter, Processor, RecoveryBackend, RuntimeServiceId>
where
    Pool: RecoverableMempool,
    Pool::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId>,
    NetworkAdapter::Settings: Clone,
    Processor: PayloadProcessor,
    Processor::Settings: Clone,
    RecoveryBackend: RecoveryBackendTrait,
{
    type Settings =
        TxMempoolSettings<Pool::Settings, NetworkAdapter::Settings, Processor::Settings>;
    type State = TxMempoolState<
        Pool::RecoveryState,
        Pool::Settings,
        NetworkAdapter::Settings,
        Processor::Settings,
    >;
    type StateOperator = RecoveryOperator<RecoveryBackend>;
    type Message = MempoolMsg<Pool::BlockId, Pool::Item, Pool::Item, Pool::Key>;
}

#[async_trait::async_trait]
impl<Pool, NetworkAdapter, Processor, RecoveryBackend, RuntimeServiceId>
    ServiceCore<RuntimeServiceId>
    for GenericTxMempoolService<Pool, NetworkAdapter, Processor, RecoveryBackend, RuntimeServiceId>
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
    Processor: PayloadProcessor<Payload = NetworkAdapter::Payload> + Send,
    Processor::Settings: Clone + Send + Sync,
    Processor::DaSamplingService: ServiceData,
    <<Processor as PayloadProcessor>::DaSamplingService as ServiceData>::Message: Send + 'static,
    RecoveryBackend: RecoveryBackendTrait + Send,
    RuntimeServiceId: Display
        + Debug
        + Sync
        + Send
        + 'static
        + AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkAdapter::Backend, RuntimeServiceId>>
        + AsServiceId<Processor::DaSamplingService>,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        tracing::trace!(
            "Initializing TxMempoolService with initial state {:#?}",
            initial_state.pool
        );
        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let recovered_pool = initial_state.pool.map_or_else(
            || Pool::new(settings.pool.clone()),
            |recovered_pool| Pool::recover(settings.pool.clone(), recovered_pool),
        );

        Ok(Self::new(recovered_pool, service_resources_handle))
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let network_service_relay = self
            .service_resources_handle
            .overwatch_handle
            .relay::<NetworkService<_, _>>()
            .await
            .expect("Relay connection with NetworkService should succeed");

        // Queue for network messages
        let mut network_items = NetworkAdapter::new(
            self.service_resources_handle
                .settings_handle
                .notifier()
                .get_updated_settings()
                .network_adapter,
            network_service_relay.clone(),
        )
        .await
        .payload_stream()
        .await;

        let sampling_relay = self
            .service_resources_handle
            .overwatch_handle
            .relay::<Processor::DaSamplingService>()
            .await
            .expect("Relay connection with sampling service should succeed");

        self.service_resources_handle.status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &self.service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _>
        )
        .await?;

        let processor = Processor::new(
            self.service_resources_handle
                .settings_handle
                .notifier()
                .get_updated_settings()
                .processor,
            sampling_relay,
        );

        loop {
            tokio::select! {
                // Queue for relay messages
                Some(relay_msg) = self.service_resources_handle.inbound_relay.recv() => {
                    self.handle_mempool_message(relay_msg, network_service_relay.clone());
                }
                Some((key, item)) = network_items.next() => {
                    if let Err(e) =processor.process(&item).await {
                        tracing::debug!("could not process item from network due to: {e:?}");
                        continue;
                    }
                    if let Err(e) = self.pool.add_item(key, item) {
                        tracing::debug!("could not add item to the pool due to: {e}");
                        continue;
                    }
                    tracing::info!(counter.tx_mempool_pending_items = self.pool.pending_item_count());
                    self.service_resources_handle.state_updater.update(Some(self.pool.save().into()));
                }
            }
        }
    }
}

impl<Pool, NetworkAdapter, Processor, RecoveryBackend, RuntimeServiceId>
    GenericTxMempoolService<Pool, NetworkAdapter, Processor, RecoveryBackend, RuntimeServiceId>
where
    Pool: RecoverableMempool,
    Pool::Item: Clone + Send + 'static,
    Pool::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId, Payload = Pool::Item> + Send,
    NetworkAdapter::Settings: Clone + Send + 'static,
    Processor: PayloadProcessor<Payload = NetworkAdapter::Payload>,
    Processor::Settings: Clone,
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
        network_relay: OutboundRelay<BackendNetworkMsg<NetworkAdapter::Backend, RuntimeServiceId>>,
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
                            .service_resources_handle
                            .settings_handle
                            .notifier()
                            .get_updated_settings()
                            .network_adapter;
                        self.service_resources_handle
                            .state_updater
                            .update(Some(self.pool.save().into()));
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
