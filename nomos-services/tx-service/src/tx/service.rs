/// Re-export for `OpenAPI`
#[cfg(feature = "openapi")]
pub mod openapi {
    pub use crate::backend::Status;
}

use std::{
    collections::BTreeSet,
    fmt::{Debug, Display},
    marker::PhantomData,
    pin::Pin,
    time::Duration,
};

use futures::{StreamExt as _, stream::FuturesUnordered};
use nomos_da_sampling::backend::kzgrs::KzgrsSamplingBackend;
use nomos_network::{NetworkService, message::BackendNetworkMsg};
use nomos_storage::StorageService;
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceCore, ServiceData, relay::OutboundRelay},
};
use services_utils::{
    overwatch::{
        JsonFileBackend, RecoveryOperator,
        recovery::operators::RecoveryBackend as RecoveryBackendTrait,
    },
    wait_until_services_are_ready,
};
use tokio::sync::oneshot;

use crate::{
    MempoolMetrics, MempoolMsg, backend,
    backend::{MemPool as MemPoolTrait, RecoverableMempool},
    network::NetworkAdapter as NetworkAdapterTrait,
    processor::{PayloadProcessor, tx::SignedTxProcessor},
    storage::MempoolStorageAdapter,
    tx::{settings::TxMempoolSettings, state::TxMempoolState},
};

pub type DaSamplingService<SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId> =
    nomos_da_sampling::DaSamplingService<
        KzgrsSamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    >;

type MempoolStateUpdater<Pool, NetworkAdapter, Processor, RuntimeServiceId> =
    overwatch::services::state::StateUpdater<
        Option<
            TxMempoolState<
                <Pool as RecoverableMempool>::RecoveryState,
                <Pool as MemPoolTrait>::Settings,
                <NetworkAdapter as NetworkAdapterTrait<RuntimeServiceId>>::Settings,
                <Processor as PayloadProcessor>::Settings,
            >,
        >,
    >;

type TxMempoolRecoveryState<Pool, NetworkAdapter, Processor, RuntimeServiceId> = TxMempoolState<
    <Pool as RecoverableMempool>::RecoveryState,
    <Pool as MemPoolTrait>::Settings,
    <NetworkAdapter as NetworkAdapterTrait<RuntimeServiceId>>::Settings,
    <Processor as PayloadProcessor>::Settings,
>;

type TxMempoolRecoverySettings<Pool, NetworkAdapter, Processor, RuntimeServiceId> =
    TxMempoolSettings<
        <Pool as MemPoolTrait>::Settings,
        <NetworkAdapter as NetworkAdapterTrait<RuntimeServiceId>>::Settings,
        <Processor as PayloadProcessor>::Settings,
    >;

type TxMempoolRecoveryBackend<Pool, NetworkAdapter, Processor, RuntimeServiceId> = JsonFileBackend<
    TxMempoolRecoveryState<Pool, NetworkAdapter, Processor, RuntimeServiceId>,
    TxMempoolRecoverySettings<Pool, NetworkAdapter, Processor, RuntimeServiceId>,
>;

/// A tx mempool service that uses a [`JsonFileBackend`] as a recovery
/// mechanism.
pub type TxMempoolService<
    MempoolNetworkAdapter,
    SamplingNetworkAdapter,
    SamplingStorage,
    Pool,
    StorageAdapter,
    RuntimeServiceId,
> = GenericTxMempoolService<
    Pool,
    MempoolNetworkAdapter,
    SignedTxProcessor<DaSamplingService<SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>>,
    TxMempoolRecoveryBackend<
        Pool,
        MempoolNetworkAdapter,
        SignedTxProcessor<
            DaSamplingService<SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>,
        >,
        RuntimeServiceId,
    >,
    StorageAdapter,
    RuntimeServiceId,
>;

/// A generic tx mempool service which wraps around a mempool, a network
/// adapter, and a recovery backend.
pub struct GenericTxMempoolService<
    Pool,
    NetworkAdapter,
    Processor,
    RecoveryBackend,
    StorageAdapter,
    RuntimeServiceId,
> where
    Pool: MemPoolTrait<Storage = StorageAdapter> + RecoverableMempool + Send + Sync,
    StorageAdapter: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    <Pool as MemPoolTrait>::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId> + Send + Sync,
    NetworkAdapter::Settings: Clone,
    Processor: PayloadProcessor + Send + Sync,
    Processor::Settings: Clone,
    RecoveryBackend: RecoveryBackendTrait + Send + Sync,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    initial_state: <Self as ServiceData>::State,
    _phantom: PhantomData<(
        Pool,
        NetworkAdapter,
        Processor,
        RecoveryBackend,
        StorageAdapter,
    )>,
}

impl<Pool, NetworkAdapter, Processor, RecoveryBackend, StorageAdapter, RuntimeServiceId>
    GenericTxMempoolService<
        Pool,
        NetworkAdapter,
        Processor,
        RecoveryBackend,
        StorageAdapter,
        RuntimeServiceId,
    >
where
    Pool: MemPoolTrait<Storage = StorageAdapter> + RecoverableMempool + Send + Sync,
    StorageAdapter: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    <Pool as MemPoolTrait>::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId> + Send + Sync,
    NetworkAdapter::Settings: Clone,
    Processor: PayloadProcessor + Send + Sync,
    Processor::Settings: Clone,
    RecoveryBackend: RecoveryBackendTrait + Send + Sync,
{
    pub const fn new(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        initial_state: <Self as ServiceData>::State,
    ) -> Self {
        Self {
            service_resources_handle,
            initial_state,
            _phantom: PhantomData,
        }
    }
}

impl<Pool, NetworkAdapter, Processor, RecoveryBackend, StorageAdapter, RuntimeServiceId> ServiceData
    for GenericTxMempoolService<
        Pool,
        NetworkAdapter,
        Processor,
        RecoveryBackend,
        StorageAdapter,
        RuntimeServiceId,
    >
where
    Pool: MemPoolTrait<Storage = StorageAdapter> + RecoverableMempool + Send + Sync,
    StorageAdapter: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    <Pool as MemPoolTrait>::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId> + Send + Sync,
    NetworkAdapter::Settings: Clone,
    Processor: PayloadProcessor + Send + Sync,
    Processor::Settings: Clone,
    RecoveryBackend: RecoveryBackendTrait + Send + Sync,
{
    type Settings = TxMempoolSettings<
        <Pool as MemPoolTrait>::Settings,
        NetworkAdapter::Settings,
        Processor::Settings,
    >;
    type State = TxMempoolState<
        <Pool as RecoverableMempool>::RecoveryState,
        <Pool as MemPoolTrait>::Settings,
        NetworkAdapter::Settings,
        Processor::Settings,
    >;
    type StateOperator = RecoveryOperator<RecoveryBackend>;
    type Message = MempoolMsg<Pool::BlockId, Pool::Item, Pool::Item, Pool::Key>;
}

#[async_trait::async_trait]
impl<Pool, NetworkAdapter, Processor, RecoveryBackend, StorageAdapter, RuntimeServiceId>
    ServiceCore<RuntimeServiceId>
    for GenericTxMempoolService<
        Pool,
        NetworkAdapter,
        Processor,
        RecoveryBackend,
        StorageAdapter,
        RuntimeServiceId,
    >
where
    Pool: MemPoolTrait<Storage = StorageAdapter> + RecoverableMempool + Send + Sync,
    StorageAdapter: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    <Pool as RecoverableMempool>::RecoveryState: Debug + Send + Sync,
    Pool::Item: Clone + Send + 'static,
    Pool::Settings: Clone + Sync + Send,
    NetworkAdapter:
        NetworkAdapterTrait<RuntimeServiceId, Payload = Pool::Item, Key = Pool::Key> + Send + Sync,
    NetworkAdapter::Settings: Clone + Send + Sync + 'static,
    Processor: PayloadProcessor<Payload = NetworkAdapter::Payload> + Send + Sync,
    Processor::Settings: Clone + Send + Sync,
    Processor::DaSamplingService: ServiceData,
    Processor::Error: Send + Sync,
    <<Processor as PayloadProcessor>::DaSamplingService as ServiceData>::Message: Send + 'static,
    RecoveryBackend: RecoveryBackendTrait + Send + Sync,
    RuntimeServiceId: Display
        + Debug
        + Sync
        + Send
        + 'static
        + AsServiceId<Self>
        + AsServiceId<NetworkService<NetworkAdapter::Backend, RuntimeServiceId>>
        + AsServiceId<Processor::DaSamplingService>
        + AsServiceId<
            StorageService<
                <StorageAdapter as MempoolStorageAdapter<RuntimeServiceId>>::Backend,
                RuntimeServiceId,
            >,
        >,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        tracing::trace!(
            "Initializing TxMempoolService with initial state {:#?}",
            initial_state.pool
        );
        Ok(Self::new(service_resources_handle, initial_state))
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let settings_handle = &self.service_resources_handle.settings_handle;
        let settings = settings_handle.notifier().get_updated_settings();

        let overwatch_handle = &self.service_resources_handle.overwatch_handle;

        let storage_relay = overwatch_handle
            .relay::<StorageService<
                <StorageAdapter as MempoolStorageAdapter<RuntimeServiceId>>::Backend,
                RuntimeServiceId,
            >>()
            .await
            .expect("Storage service relay should be available");

        let storage_adapter =
            <StorageAdapter as MempoolStorageAdapter<RuntimeServiceId>>::new(storage_relay);

        let pool_state = self.initial_state.pool.take();

        let mut pool = match pool_state {
            None => <Pool as MemPoolTrait>::new(settings.pool.clone(), storage_adapter),
            Some(recovered_pool_state) => <Pool as RecoverableMempool>::recover(
                settings.pool.clone(),
                recovered_pool_state,
                storage_adapter,
            ),
        };

        let network_service_relay = overwatch_handle
            .relay::<NetworkService<_, _>>()
            .await
            .expect("Relay connection with NetworkService should succeed");

        // Queue for network messages
        let mut network_items = NetworkAdapter::new(
            settings_handle
                .notifier()
                .get_updated_settings()
                .network_adapter,
            network_service_relay.clone(),
        )
        .await
        .payload_stream()
        .await;

        let sampling_relay = overwatch_handle
            .relay::<Processor::DaSamplingService>()
            .await
            .expect("Relay connection with sampling service should succeed");

        self.service_resources_handle.status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        let mut processor_tasks = FuturesUnordered::new();

        let processor = Processor::new(
            settings_handle.notifier().get_updated_settings().processor,
            sampling_relay,
        );

        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _>
        )
        .await?;

        self.run_event_loop(
            &mut pool,
            network_service_relay,
            &mut network_items,
            &processor,
            &mut processor_tasks,
        )
        .await
    }
}

impl<Pool, NetworkAdapter, Processor, RecoveryBackend, StorageAdapter, RuntimeServiceId>
    GenericTxMempoolService<
        Pool,
        NetworkAdapter,
        Processor,
        RecoveryBackend,
        StorageAdapter,
        RuntimeServiceId,
    >
where
    Pool: MemPoolTrait<Storage = StorageAdapter> + RecoverableMempool + Send + Sync,
    StorageAdapter: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    Pool::Item: Clone + Send + 'static,
    Pool::Settings: Clone,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId, Payload = Pool::Item> + Send + Sync,
    NetworkAdapter::Settings: Clone + Send + 'static,
    Processor: PayloadProcessor<Payload = NetworkAdapter::Payload> + Send + Sync,
    Processor::Settings: Clone,
    Processor::Error: Send + Sync,
    RecoveryBackend: RecoveryBackendTrait + Send + Sync,
    RuntimeServiceId: 'static,
{
    async fn run_event_loop(
        &mut self,
        pool: &mut Pool,
        network_service_relay: OutboundRelay<
            BackendNetworkMsg<NetworkAdapter::Backend, RuntimeServiceId>,
        >,
        network_items: &mut Box<dyn futures::Stream<Item = (Pool::Key, Pool::Item)> + Unpin + Send>,
        processor: &Processor,
        processor_tasks: &mut FuturesUnordered<
            futures::future::BoxFuture<'static, Result<(), Processor::Error>>,
        >,
    ) -> Result<(), overwatch::DynError>
    where
        Pool::Settings: Send + Sync,
        NetworkAdapter::Settings: Send + Sync,
        Processor::Settings: Send + Sync,
    {
        loop {
            tokio::select! {
                // Queue for relay messages
                Some(relay_msg) = self.service_resources_handle.inbound_relay.recv() => {
                    let state_updater = self.service_resources_handle.state_updater.clone();
                    let settings = self
                        .service_resources_handle
                        .settings_handle
                        .notifier()
                        .get_updated_settings()
                        .network_adapter;

                    Self::handle_mempool_message(pool, relay_msg, network_service_relay.clone(), state_updater, settings).await;
                }
                Some((key, item)) = network_items.next() => {
                    Self::handle_network_item(pool, key, item, processor, processor_tasks, &self.service_resources_handle.state_updater).await;
                }
                Some(result) = processor_tasks.next() => {
                    if let Err(e) = result {
                        tracing::error!("coulnd not complete processor task: {e}");
                    }
                },
            }
        }
    }

    async fn handle_mempool_message(
        pool: &mut Pool,
        message: MempoolMsg<Pool::BlockId, Pool::Item, Pool::Item, Pool::Key>,
        network_relay: OutboundRelay<BackendNetworkMsg<NetworkAdapter::Backend, RuntimeServiceId>>,
        state_updater: MempoolStateUpdater<Pool, NetworkAdapter, Processor, RuntimeServiceId>,
        settings: NetworkAdapter::Settings,
    ) where
        Pool::Settings: Send + Sync,
        NetworkAdapter::Settings: Send + Sync,
        Processor::Settings: Send + Sync,
    {
        match message {
            MempoolMsg::Add {
                payload,
                key,
                reply_channel,
            } => {
                Self::handle_add_message(
                    pool,
                    key,
                    payload,
                    reply_channel,
                    network_relay,
                    state_updater,
                    settings,
                )
                .await;
            }
            MempoolMsg::View {
                ancestor_hint,
                reply_channel,
            } => {
                Self::handle_view_message(pool, ancestor_hint, reply_channel).await;
            }
            MempoolMsg::GetTransactionsByHashes {
                hashes,
                reply_channel,
            } => {
                Self::handle_get_transactions_message(pool, hashes, reply_channel).await;
            }
            MempoolMsg::MarkInBlock { ids, block } => {
                pool.mark_in_block(&ids, block);
            }
            MempoolMsg::Prune { ids } => {
                pool.prune(&ids).await;
            }
            MempoolMsg::Metrics { reply_channel } => {
                Self::handle_metrics_message(pool, reply_channel);
            }
            MempoolMsg::Status {
                items,
                reply_channel,
            } => {
                Self::handle_status_message(pool, &items, reply_channel);
            }
        }
    }

    async fn handle_add_message(
        pool: &mut Pool,
        key: Pool::Key,
        item: Pool::Item,
        reply_channel: oneshot::Sender<Result<(), backend::MempoolError>>,
        network_relay: OutboundRelay<BackendNetworkMsg<NetworkAdapter::Backend, RuntimeServiceId>>,
        state_updater: MempoolStateUpdater<Pool, NetworkAdapter, Processor, RuntimeServiceId>,
        settings: NetworkAdapter::Settings,
    ) where
        Pool::Settings: Send + Sync,
        NetworkAdapter::Settings: Send + Sync,
        Processor::Settings: Send + Sync,
    {
        let item_for_broadcast = item.clone();

        match pool.add_item(key, item).await {
            Ok(_id) => {
                Self::handle_add_success(
                    pool,
                    &state_updater,
                    settings,
                    network_relay,
                    item_for_broadcast,
                    reply_channel,
                );
            }
            Err(e) => {
                Self::handle_add_error(e, reply_channel);
            }
        }
    }

    async fn handle_view_message(
        pool: &Pool,
        ancestor_hint: Pool::BlockId,
        reply_channel: oneshot::Sender<Pin<Box<dyn futures::Stream<Item = Pool::Item> + Send>>>,
    ) {
        let pending_items = pool.pending_item_count();
        tracing::trace!(pending_items, "Handling mempool View message");

        let items = pool
            .view(ancestor_hint)
            .await
            .unwrap_or_else(|_| Box::pin(futures::stream::iter(Vec::new())));

        if let Err(_e) = reply_channel.send(Box::pin(items)) {
            tracing::debug!("Failed to send view reply");
        }
    }

    fn handle_metrics_message(pool: &Pool, reply_channel: oneshot::Sender<MempoolMetrics>) {
        let info = MempoolMetrics {
            pending_items: pool.pending_item_count(),
            last_item_timestamp: pool.last_item_timestamp(),
        };

        if let Err(_e) = reply_channel.send(info) {
            tracing::debug!("Failed to send metrics reply");
        }
    }

    fn handle_status_message(
        pool: &Pool,
        items: &[Pool::Key],
        reply_channel: oneshot::Sender<Vec<backend::Status<Pool::BlockId>>>,
    ) {
        let statuses = pool.status(items);

        if let Err(_e) = reply_channel.send(statuses) {
            tracing::debug!("Failed to send status reply");
        }
    }

    async fn handle_get_transactions_message(
        pool: &Pool,
        hashes: BTreeSet<Pool::Key>,
        reply_channel: oneshot::Sender<Pin<Box<dyn futures::Stream<Item = Pool::Item> + Send>>>,
    ) {
        let transactions = pool
            .get_items_by_keys(hashes)
            .await
            .unwrap_or_else(|_| Box::pin(futures::stream::iter(Vec::new())));

        if let Err(_e) = reply_channel.send(transactions) {
            tracing::debug!("Failed to send transactions reply");
        }
    }

    fn handle_add_success(
        pool: &Pool,
        state_updater: &MempoolStateUpdater<Pool, NetworkAdapter, Processor, RuntimeServiceId>,
        settings: NetworkAdapter::Settings,
        network_relay: OutboundRelay<BackendNetworkMsg<NetworkAdapter::Backend, RuntimeServiceId>>,
        item_for_broadcast: Pool::Item,
        reply_channel: oneshot::Sender<Result<(), backend::MempoolError>>,
    ) {
        state_updater.update(Some(<Pool as RecoverableMempool>::save(pool).into()));

        tokio::spawn(async move {
            let adapter = NetworkAdapter::new(settings, network_relay).await;
            adapter.send(item_for_broadcast).await;
        });

        if let Err(e) = reply_channel.send(Ok(())) {
            tracing::debug!("Failed to send add reply: {:?}", e);
        }
    }

    fn handle_add_error(
        error: backend::MempoolError,
        reply_channel: oneshot::Sender<Result<(), backend::MempoolError>>,
    ) {
        tracing::debug!("Could not add item to the pool: {}", error);
        if let Err(e) = reply_channel.send(Err(error)) {
            tracing::debug!("Failed to send error reply: {:?}", e);
        }
    }

    async fn handle_network_item(
        pool: &mut Pool,
        key: Pool::Key,
        item: Pool::Item,
        processor: &Processor,
        processor_tasks: &mut FuturesUnordered<
            futures::future::BoxFuture<'static, Result<(), Processor::Error>>,
        >,
        state_updater: &MempoolStateUpdater<Pool, NetworkAdapter, Processor, RuntimeServiceId>,
    ) where
        Pool::Settings: Send + Sync,
        NetworkAdapter::Settings: Send + Sync,
        Processor::Settings: Send + Sync,
    {
        match processor.process(&item).await {
            Ok(new_tasks) => {
                processor_tasks.extend(new_tasks);
            }
            Err(e) => {
                tracing::debug!("could not process item from network due to: {e:?}");
                return;
            }
        }

        if let Err(e) = pool.add_item(key, item).await {
            tracing::debug!("could not add item to the pool due to: {e}");
            return;
        }

        tracing::info!(counter.tx_mempool_pending_items = pool.pending_item_count());

        state_updater.update(Some(<Pool as RecoverableMempool>::save(pool).into()));
    }
}
