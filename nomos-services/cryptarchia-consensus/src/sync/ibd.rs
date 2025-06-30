use std::{collections::HashSet, fmt::Debug};

use futures::StreamExt as _;
use nomos_core::{
    block::Block,
    da::blob::{info::DispersedBlobInfo, metadata::Metadata as BlobMetadata, BlobSelect},
    header::HeaderId,
    mantle::{Transaction, TxSelect},
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_mempool::{backend::RecoverableMempool, network::NetworkAdapter as MempoolAdapter};
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend};
use overwatch::DynError;
use rand::{RngCore, SeedableRng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    blend, network::NetworkAdapter, relays::CryptarchiaConsensusRelays, Cryptarchia,
    CryptarchiaConsensus,
};

const LOG_TARGET: &str = "cryptarchia::service::sync::ibd";

#[expect(clippy::type_complexity, reason = "amount of generics.")]
pub struct InitialBlockDownload<
    CryptarchiaState,
    BlendAdapter,
    BS,
    ClPool,
    ClPoolAdapter,
    DaPool,
    DaPoolAdapter,
    NetAdapter,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingRng,
    SamplingStorage,
    Storage,
    TxS,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    TimeBackend,
    ApiAdapter,
    RuntimeServiceId,
> {
    _phantom: std::marker::PhantomData<(
        CryptarchiaState,
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetAdapter,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        Storage,
        TxS,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    )>,
}

impl<
        CryptarchiaState,
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetAdapter,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        Storage,
        TxS,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
    InitialBlockDownload<
        CryptarchiaState,
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetAdapter,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        Storage,
        TxS,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    CryptarchiaState: cryptarchia_engine::CryptarchiaState,
    BlendAdapter: blend::BlendAdapter<RuntimeServiceId> + Clone + Send + Sync + 'static,
    BlendAdapter::Settings: Send,
    NetAdapter: NetworkAdapter<RuntimeServiceId, Tx = ClPool::Item, BlobCertificate = DaPool::Item>
        + Clone
        + Send
        + Sync
        + 'static,
    NetAdapter::Settings: Send,
    NetAdapter::PeerId: Debug + Clone + Send + Sync + 'static,
    ClPool: RecoverableMempool<BlockId = HeaderId> + Send + Sync + 'static,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone + Send + Sync + 'static,
    ClPool::Item: Transaction<Hash = ClPool::Key>
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    ClPool::Key: Debug + Send + Sync,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>
        + Send
        + Sync
        + 'static,
    DaPool: RecoverableMempool<BlockId = HeaderId, Key = SamplingBackend::BlobId>
        + Send
        + Sync
        + 'static,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Settings: Clone + Send + Sync + 'static,
    DaPool::Item: DispersedBlobInfo<BlobId = DaPool::Key>
        + BlobMetadata
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key> + Send + Sync + 'static,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item> + Clone + Send + Sync + 'static,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item> + Clone + Send + Sync + 'static,
    BS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item, DaPool::Item>> + TryInto<Block<ClPool::Item, DaPool::Item>>,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + Ord + Send + Sync + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingRng: SeedableRng + RngCore,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
{
    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusRelays amount of generics."
    )]
    pub async fn run(
        mut cryptarchia: Cryptarchia<CryptarchiaState>,
        network_adapter: NetAdapter,
        relays: &CryptarchiaConsensusRelays<
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
    ) -> Result<Cryptarchia<CryptarchiaState>, DynError> {
        // Run IBD only when a set of IBD peers are configured.
        // Return an error if none of them are not available.
        //
        // TODO: Currently, getting connected peers from network service
        // because `initial_peers` are configured for the network service.
        // However, it’s unclear whether the initial peers were empty or just
        // unreachable. So, consider moving the `initial_peers` to the consensus
        // service settings and renaming it to `ibd_peers`.
        // Then, try to connect to the `ibd_peers` here.
        // If none of them is connected, return an error.
        let peers = network_adapter.connected_peers().await?;
        tracing::info!(
            target: LOG_TARGET,
            "Starting IBD with {} peers: {:?}",
            peers.len(), peers
        );

        // TODO: Run with multiple peers in parallel.
        // For now, we run with the first peer only for easy debugging.
        if let Some(peer) = peers.iter().next() {
            cryptarchia =
                Self::run_with_peer(peer.clone(), cryptarchia, network_adapter, relays).await?;
        }

        Ok(cryptarchia)
    }

    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusRelays amount of generics."
    )]
    async fn run_with_peer(
        peer: NetAdapter::PeerId,
        mut cryptarchia: Cryptarchia<CryptarchiaState>,
        network_adapter: NetAdapter,
        relays: &CryptarchiaConsensusRelays<
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
    ) -> Result<Cryptarchia<CryptarchiaState>, DynError> {
        let mut latest_downloaded_block: Option<HeaderId> = None;
        loop {
            // Request the latest tip from the peer for each iteration
            let tip = network_adapter.request_tip(peer.clone()).await?;
            tracing::debug!(
                target: LOG_TARGET,
                "A tip received from peer {peer:?}. Start downloading up to the tip: {tip:?}",
            );

            // Download blocks from the peer up to the tip.
            // The tip may not be reached because of the response limit set by the peer.
            let mut block_stream = network_adapter
                .request_blocks_from_peer(
                    peer.clone(),
                    tip,
                    cryptarchia.tip(),
                    cryptarchia.lib(),
                    // Provide the latest downloaded block to avoid downloading duplicate blocks.
                    latest_downloaded_block
                        .map(|id| HashSet::from([id]))
                        .unwrap_or_default(),
                )
                .await?;

            while let Some(result) = block_stream.next().await {
                let block = result?;
                tracing::trace!(
                    target: LOG_TARGET,
                    "A block received from peer {peer:?}: {:?}", block.header().id()
                );
                latest_downloaded_block = Some(block.header().id());
                // TODO: Abort downloading if `process_block` returns an error.
                // This requires refactoring of the `process_block` method to return a `Result`.
                cryptarchia =
                    CryptarchiaConsensus::<
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
                    >::process_block(cryptarchia, block, relays, None, true)
                    .await;
            }

            // If the tip has been downloaded and applied, downloading is complete.
            if cryptarchia.has_block(&tip) {
                tracing::info!(
                    target: LOG_TARGET,
                    "IBD with peer {peer:?} is complete: tip:{tip:?}"
                );
                return Ok(cryptarchia);
            }
        }
    }
}

// TODO: Add tests. It's hard to test without abstracting `CryptarchiaConsensus`
