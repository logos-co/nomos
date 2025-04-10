use core::fmt::Debug;
use std::{hash::Hash, marker::PhantomData};

use cryptarchia_engine::Slot;
use cryptarchia_sync::adapter::CryptarchiaAdapterError;
use nomos_core::{
    block::Block,
    da::blob::{info::DispersedBlobInfo, metadata::Metadata as BlobMetadata, BlobSelect},
    header::HeaderId,
    tx::{Transaction, TxSelect},
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_mempool::{backend::RecoverableMempool, network::NetworkAdapter as MempoolAdapter};
use nomos_storage::backends::StorageBackend;
use rand::{RngCore, SeedableRng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{
    blend, leadership, network::NetworkAdapter, relays::CryptarchiaConsensusRelays, Cryptarchia,
    CryptarchiaConsensus, Error,
};

#[expect(
    clippy::type_complexity,
    reason = "CryptarchiaConsensus amount of generics."
)]
pub struct CryptarchiaSyncAdapter<
    NetAdapter,
    BlendAdapter,
    ClPool,
    ClPoolAdapter,
    DaPool,
    DaPoolAdapter,
    TxS,
    BS,
    Storage,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingRng,
    SamplingStorage,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    TimeBackend,
    ApiAdapter,
    RuntimeServiceId,
> where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Backend: 'static,
    NetAdapter::Settings: Send,
    BlendAdapter: blend::BlendAdapter<RuntimeServiceId>,
    BlendAdapter::Settings: Send,
    ClPool: RecoverableMempool<BlockId = HeaderId>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone,
    ClPool::Item: Clone + Eq + Hash + Debug + 'static,
    ClPool::Key: Debug + 'static,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    DaPool: RecoverableMempool<BlockId = HeaderId>,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Item: Clone + Eq + Hash + Debug + 'static,
    DaPool::Key: Debug + 'static,
    DaPool::Settings: Clone,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item>,
    BS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = DaPool::Key> + Send + 'static,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingRng: SeedableRng + RngCore,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + 'static,
    DaVerifierBackend::Settings: Clone,
{
    cryptarchia: Option<Cryptarchia>,
    leader: leadership::Leader,
    relays: CryptarchiaConsensusRelays<
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetAdapter,
        SamplingBackend,
        SamplingRng,
        Storage,
        TxS,
        DaVerifierBackend,
        RuntimeServiceId,
    >,
    block_broadcaster: broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    _marker: PhantomData<(
        ApiAdapter,
        SamplingNetworkAdapter,
        SamplingStorage,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
    )>,
}

#[expect(
    clippy::type_complexity,
    reason = "CryptarchiaConsensus amount of generics."
)]
impl<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
    CryptarchiaSyncAdapter<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Backend: 'static,
    NetAdapter::Settings: Send,
    BlendAdapter: blend::BlendAdapter<RuntimeServiceId>,
    BlendAdapter::Settings: Send,
    ClPool: RecoverableMempool<BlockId = HeaderId>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone,
    ClPool::Item: Clone + Eq + Hash + Debug + 'static,
    ClPool::Key: Debug + 'static,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    DaPool: RecoverableMempool<BlockId = HeaderId>,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Item: Clone + Eq + Hash + Debug + 'static,
    DaPool::Key: Debug + 'static,
    DaPool::Settings: Clone,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item>,
    BS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = DaPool::Key> + Send + 'static,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingRng: SeedableRng + RngCore,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + 'static,
    DaVerifierBackend::Settings: Clone,
{
    pub const fn new(
        cryptarchia: Cryptarchia,
        leader: leadership::Leader,
        relays: CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_broadcaster: broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    ) -> Self {
        Self {
            cryptarchia: Some(cryptarchia),
            leader,
            relays,
            block_broadcaster,
            _marker: PhantomData::<(
                ApiAdapter,
                SamplingNetworkAdapter,
                SamplingStorage,
                DaVerifierNetwork,
                DaVerifierStorage,
                TimeBackend,
            )>,
        }
    }

    pub fn take(
        self,
    ) -> (
        Cryptarchia,
        leadership::Leader,
        CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
    ) {
        (
            self.cryptarchia.expect("Cryptarchia is not set"),
            self.leader,
            self.relays,
        )
    }
}

#[async_trait::async_trait]
impl<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    > cryptarchia_sync::adapter::CryptarchiaAdapter
    for CryptarchiaSyncAdapter<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    NetAdapter: NetworkAdapter<RuntimeServiceId> + Clone + Send + Sync + 'static,
    NetAdapter::Settings: Send,
    BlendAdapter: blend::BlendAdapter<RuntimeServiceId> + Clone + Send + Sync + 'static,
    BlendAdapter::Settings: Send,
    ClPool: RecoverableMempool<BlockId = HeaderId> + Send + Sync + 'static,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone + Send + Sync + 'static,
    ClPool::Item: Transaction<Hash = ClPool::Key>
        + Debug
        + Clone
        + Eq
        + Hash
        + Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
    ClPool::Key: Debug + Send + Sync,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>
        + Send
        + Sync
        + 'static,
    DaPool::Item: DispersedBlobInfo<BlobId = DaPool::Key>
        + BlobMetadata
        + Debug
        + Clone
        + Eq
        + Hash
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    DaPool: RecoverableMempool<BlockId = HeaderId, Key = SamplingBackend::BlobId>
        + Send
        + Sync
        + 'static,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Settings: Clone + Send + Sync + 'static,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key> + Send + Sync + 'static,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item> + Clone + Send + Sync + 'static,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item> + Clone + Send + Sync + 'static,
    BS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng> + Send + 'static,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + Ord + Send + Sync + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send,
    SamplingRng: SeedableRng + RngCore,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend + Send,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
{
    type Block = Block<ClPool::Item, DaPool::Item>;

    async fn process_block(&mut self, block: Self::Block) -> Result<(), CryptarchiaAdapterError> {
        let result = CryptarchiaConsensus::<
            NetAdapter,
            BlendAdapter,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            TxS,
            BS,
            Storage,
            SamplingBackend,
            SamplingNetworkAdapter,
            SamplingRng,
            SamplingStorage,
            DaVerifierBackend,
            DaVerifierNetwork,
            DaVerifierStorage,
            TimeBackend,
            ApiAdapter,
            RuntimeServiceId,
        >::process_block(
            self.cryptarchia.take().unwrap(),
            &mut self.leader,
            block,
            &self.relays,
            &mut self.block_broadcaster,
            true,
        )
        .await;

        match result {
            Ok(cryptarchia) => {
                self.cryptarchia = Some(cryptarchia);
                Ok(())
            }
            Err((e, cryptarchia)) => {
                self.cryptarchia = Some(cryptarchia);
                match e {
                    Error::Ledger(nomos_ledger::LedgerError::ParentNotFound(_))
                    | Error::Consensus(cryptarchia_engine::Error::ParentMissing(_)) => {
                        Err(CryptarchiaAdapterError::ParentNotFound)
                    }
                    e => Err(CryptarchiaAdapterError::InvalidBlock(Box::new(e))),
                }
            }
        }
    }

    fn tip_slot(&self) -> Slot {
        self.cryptarchia
            .as_ref()
            .expect("Cryptarchia shouldn't be in use")
            .tip_state()
            .slot()
    }

    fn has_block(&self, id: &HeaderId) -> bool {
        self.cryptarchia
            .as_ref()
            .expect("Cryptarchia shouldn't be in use")
            .ledger
            .state(id)
            .is_some()
    }
}
