use std::{collections::HashMap, marker::PhantomData};

use nomos_core::{
    block::{Block, Proposal},
    header::HeaderId,
    mantle::{Transaction, TxHash},
};
use serde::{de::DeserializeOwned, Serialize};
use tracing::error;

use crate::{mempool::MempoolAdapterTrait, storage::StorageAdapter as StorageAdapterTrait, Error};

type MissingTxHashes = Vec<TxHash>;

pub struct TransactionManager<StorageAdapter, MempoolAdapter, RuntimeServiceId>
where
    StorageAdapter: StorageAdapterTrait<RuntimeServiceId> + Send + Sync,
    MempoolAdapter: MempoolAdapterTrait<StorageAdapter::Tx> + Send + Sync,
    RuntimeServiceId: Send + Sync,
{
    storage_adapter: StorageAdapter,
    mempool_adapter: MempoolAdapter,
    _phantom: PhantomData<RuntimeServiceId>,
}

impl<StorageAdapter, MempoolAdapter, RuntimeServiceId>
    TransactionManager<StorageAdapter, MempoolAdapter, RuntimeServiceId>
where
    StorageAdapter: StorageAdapterTrait<RuntimeServiceId> + Send + Sync,
    StorageAdapter::Block: TryFrom<Block<StorageAdapter::Tx>> + TryInto<Block<StorageAdapter::Tx>>,
    MempoolAdapter: MempoolAdapterTrait<StorageAdapter::Tx> + Send + Sync,
    StorageAdapter::Tx: Transaction<Hash = TxHash>
        + Clone
        + Eq
        + Send
        + Sync
        + 'static
        + Serialize
        + for<'de> DeserializeOwned,
    RuntimeServiceId: Send + Sync,
{
    pub const fn new(storage_adapter: StorageAdapter, mempool_adapter: MempoolAdapter) -> Self {
        Self {
            storage_adapter,
            mempool_adapter,
            _phantom: PhantomData,
        }
    }

    pub async fn reconstruct_block_from_proposal(
        &self,
        proposal: Proposal,
    ) -> Result<Block<StorageAdapter::Tx>, Error> {
        let hashes: Vec<TxHash> = proposal
            .references()
            .mempool_transactions
            .iter()
            .map(|tx_ref| TxHash::from(*tx_ref))
            .collect();

        let transactions = self.get_transactions(&hashes).await?;

        Self::build_block_from_proposal_and_transactions(&proposal, transactions)
    }

    pub async fn store_block_transactions(
        &self,
        transactions: impl Iterator<Item = StorageAdapter::Tx> + Send,
    ) -> Result<(), String> {
        let storage_transactions: Vec<StorageAdapter::Tx> = transactions.collect();

        self.storage_adapter
            .store_transactions(storage_transactions)
            .await
            .map_err(|e| format!("Failed to store transactions: {e}"))
    }

    pub async fn cleanup_transactions_from_immutable_blocks(&self, block_ids: &[HeaderId]) {
        let tx_hashes_to_cleanup: Vec<TxHash> = {
            let mut hashes = Vec::new();
            for block_id in block_ids {
                if let Some(storage_block) = self.storage_adapter.get_block(block_id).await {
                    if let Ok(block) = storage_block.try_into() {
                        let block: Block<StorageAdapter::Tx> = block;
                        hashes.extend(block.transactions().map(Transaction::hash));
                    }
                }
            }
            hashes
        };

        if !tx_hashes_to_cleanup.is_empty() {
            if let Err(e) = self.remove_transactions(&tx_hashes_to_cleanup).await {
                error!(
                    "Failed to cleanup transactions from immutable blocks: {}",
                    e
                );
            }
        }
    }

    async fn get_transactions(&self, hashes: &[TxHash]) -> Result<Vec<StorageAdapter::Tx>, Error> {
        let (mempool_txs, storage_needed) = self.get_transactions_from_mempool(hashes).await?;
        let storage_txs = self.get_transactions_from_storage(&storage_needed).await?;

        let mut txs: HashMap<TxHash, StorageAdapter::Tx> = mempool_txs
            .into_iter()
            .chain(storage_txs)
            .map(|tx| (tx.hash(), tx))
            .collect();

        hashes
            .iter()
            .map(|target_hash| {
                txs.remove(target_hash).ok_or_else(|| {
                    Error::InvalidBlock(format!(
                        "Transaction {target_hash:?} not found in mempool or storage"
                    ))
                })
            })
            .collect()
    }

    async fn get_transactions_from_mempool(
        &self,
        hashes: &[TxHash],
    ) -> Result<(Vec<StorageAdapter::Tx>, MissingTxHashes), Error> {
        let mempool_txs = self
            .mempool_adapter
            .get_transactions(hashes)
            .await
            .map_err(|e| Error::InvalidBlock(format!("Failed to query mempool: {e}")))?;

        let found_hashes: std::collections::HashSet<TxHash> =
            mempool_txs.iter().map(Transaction::hash).collect();

        let storage_needed: MissingTxHashes = hashes
            .iter()
            .filter(|&hash| !found_hashes.contains(hash))
            .copied()
            .collect();

        Ok((mempool_txs, storage_needed))
    }

    async fn get_transactions_from_storage(
        &self,
        hashes: &[TxHash],
    ) -> Result<Vec<StorageAdapter::Tx>, Error> {
        if hashes.is_empty() {
            return Ok(Vec::new());
        }

        self.storage_adapter
            .get_transactions(hashes)
            .await
            .map_err(|e| {
                Error::InvalidBlock(format!("Failed to retrieve transactions from storage: {e}"))
            })
    }

    fn build_block_from_proposal_and_transactions(
        proposal: &Proposal,
        transactions: Vec<StorageAdapter::Tx>,
    ) -> Result<Block<StorageAdapter::Tx>, Error> {
        let header = proposal.header().clone();
        let service_reward = proposal.references().service_reward;
        let signature = *proposal.signature();

        let block = Block::new(header, transactions, service_reward, signature);
        block
            .validate()
            .map_err(|e| Error::InvalidBlock(format!("Reconstructed block is invalid: {e}")))?;

        Ok(block)
    }

    async fn remove_transactions(&self, tx_hashes: &[TxHash]) -> Result<(), String> {
        self.storage_adapter
            .remove_transactions(tx_hashes)
            .await
            .map_err(|e| format!("Failed to remove transactions: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cryptarchia_engine::Slot;
    use groth16::Fr;
    use nomos_core::{
        mantle::{gas::Gas, ledger::Tx as LedgerTx, MantleTx, SignedMantleTx},
        proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
    };
    use tokio::sync::Mutex;

    use super::*;
    use crate::sync::block_provider::tests::TestEnv;

    fn create_test_block_with_txs(transactions: Vec<SignedMantleTx>) -> Block<SignedMantleTx> {
        use ed25519_dalek::SigningKey;
        use nomos_core::header::Header;

        let dummy_signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let dummy_proof = TestEnv::make_test_proof();

        let header = Header::new(
            HeaderId::from([0; 32]),
            [0; 32].into(),
            Slot::genesis(),
            dummy_proof,
        );

        let signature = header
            .sign(&dummy_signing_key)
            .expect("Failed to sign header");

        Block::new(header, transactions, None, signature)
    }

    struct MockStorageAdapter {
        transactions: Mutex<HashMap<TxHash, SignedMantleTx>>,
        blocks: Mutex<HashMap<HeaderId, Block<SignedMantleTx>>>,
    }

    impl MockStorageAdapter {
        fn new() -> Self {
            Self::with_transactions(vec![])
        }

        fn with_transactions(transactions: Vec<SignedMantleTx>) -> Self {
            let transaction_map = transactions.into_iter().map(|tx| (tx.hash(), tx)).collect();
            Self {
                transactions: Mutex::new(transaction_map),
                blocks: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl StorageAdapterTrait<()> for MockStorageAdapter {
        type Backend = nomos_storage::backends::rocksdb::RocksBackend;
        type Block = Block<SignedMantleTx>;
        type Tx = SignedMantleTx;

        async fn new(
            _: overwatch::services::relay::OutboundRelay<nomos_storage::StorageMsg<Self::Backend>>,
        ) -> Self {
            Self::new()
        }

        async fn get_block(&self, key: &HeaderId) -> Option<Self::Block> {
            self.blocks.lock().await.get(key).cloned()
        }

        async fn store_block(
            &self,
            header_id: HeaderId,
            block: Self::Block,
        ) -> Result<(), overwatch::DynError> {
            self.blocks.lock().await.insert(header_id, block);
            Ok(())
        }

        async fn remove_block(
            &self,
            header_id: HeaderId,
        ) -> Result<Option<Self::Block>, overwatch::DynError> {
            Ok(self.blocks.lock().await.remove(&header_id))
        }

        async fn store_immutable_block_ids(
            &self,
            _: BTreeMap<Slot, HeaderId>,
        ) -> Result<(), overwatch::DynError> {
            Ok(())
        }

        async fn store_transactions(
            &self,
            transactions: Vec<Self::Tx>,
        ) -> Result<(), overwatch::DynError> {
            for tx in transactions {
                self.transactions.lock().await.insert(tx.hash(), tx);
            }
            Ok(())
        }

        async fn get_transactions(
            &self,
            tx_hashes: &[TxHash],
        ) -> Result<Vec<Self::Tx>, overwatch::DynError> {
            let transactions_map = self.transactions.lock().await;
            let transactions = tx_hashes
                .iter()
                .filter_map(|&hash| transactions_map.get(&hash).cloned())
                .collect();
            Ok(transactions)
        }

        async fn remove_transactions(
            &self,
            tx_hashes: &[TxHash],
        ) -> Result<(), overwatch::DynError> {
            for &hash in tx_hashes {
                self.transactions.lock().await.remove(&hash);
            }
            Ok(())
        }
    }

    struct MockMempoolAdapter {
        transactions: Mutex<HashMap<TxHash, SignedMantleTx>>,
    }

    impl MockMempoolAdapter {
        fn new() -> Self {
            Self::with_transactions(vec![])
        }

        fn with_transactions(transactions: Vec<SignedMantleTx>) -> Self {
            let transaction_map = transactions.into_iter().map(|tx| (tx.hash(), tx)).collect();
            Self {
                transactions: Mutex::new(transaction_map),
            }
        }
    }

    #[async_trait::async_trait]
    impl MempoolAdapterTrait<SignedMantleTx> for MockMempoolAdapter {
        async fn get_transactions(
            &self,
            hashes: &[TxHash],
        ) -> Result<Vec<SignedMantleTx>, overwatch::DynError> {
            let transactions_map = self.transactions.lock().await;
            let transactions = hashes
                .iter()
                .filter_map(|&hash| transactions_map.get(&hash).cloned())
                .collect();
            Ok(transactions)
        }
    }

    fn create_test_tx(id: u32) -> SignedMantleTx {
        let public_inputs = ZkSignaturePublic {
            msg_hash: Fr::from(u64::from(id)),
            pks: vec![],
        };

        SignedMantleTx {
            mantle_tx: MantleTx {
                ops: vec![],
                ledger_tx: LedgerTx::new(vec![], vec![]),
                execution_gas_price: Gas::from(u64::from(id)),
                storage_gas_price: Gas::from(u64::from(id)),
            },
            ops_proofs: vec![],
            ledger_tx_proof: DummyZkSignature::prove(public_inputs),
        }
    }

    #[tokio::test]
    async fn test_order_preserved_with_mixed_sources() {
        let tx1 = create_test_tx(1);
        let tx2 = create_test_tx(2);
        let tx3 = create_test_tx(3);
        let tx4 = create_test_tx(4);
        let tx5 = create_test_tx(5);

        let storage_adapter =
            MockStorageAdapter::with_transactions(vec![tx1.clone(), tx3.clone(), tx5.clone()]);
        let mempool_adapter = MockMempoolAdapter::with_transactions(vec![tx2.clone(), tx4.clone()]);

        let manager = TransactionManager::new(storage_adapter, mempool_adapter);

        let request_hashes = vec![tx1.hash(), tx2.hash(), tx3.hash(), tx4.hash(), tx5.hash()];

        let result_txs = manager.get_transactions(&request_hashes).await.unwrap();

        assert_eq!(result_txs.len(), 5);
        assert_eq!(result_txs[0], tx1); // from storage
        assert_eq!(result_txs[1], tx2); // from mempool
        assert_eq!(result_txs[2], tx3); // from storage
        assert_eq!(result_txs[3], tx4); // from mempool
        assert_eq!(result_txs[4], tx5); // from storage
    }

    #[tokio::test]
    async fn test_cleanup_transactions_from_immutable_blocks() {
        let storage_adapter = MockStorageAdapter::new();
        let mempool_adapter = MockMempoolAdapter::new();

        let manager = TransactionManager::new(storage_adapter, mempool_adapter);

        let tx1 = create_test_tx(1);
        let tx2 = create_test_tx(2);
        let tx3 = create_test_tx(3);

        let test_block = create_test_block_with_txs(vec![tx1.clone(), tx2.clone()]);
        let block_id = test_block.header().id();
        manager
            .storage_adapter
            .store_transactions(vec![tx1.clone(), tx2.clone(), tx3.clone()])
            .await
            .unwrap();

        manager
            .storage_adapter
            .store_block(block_id, test_block)
            .await
            .unwrap();

        manager
            .cleanup_transactions_from_immutable_blocks(&[block_id])
            .await;

        let retrieved_txs = manager
            .storage_adapter
            .get_transactions(&[tx1.hash(), tx2.hash(), tx3.hash()])
            .await
            .unwrap();

        let retrieved_tx1_map: HashMap<TxHash, SignedMantleTx> = retrieved_txs
            .into_iter()
            .map(|tx| (tx.hash(), tx))
            .collect();

        let retrieved_tx1 = retrieved_tx1_map.get(&tx1.hash()).cloned();
        let retrieved_tx2 = retrieved_tx1_map.get(&tx2.hash()).cloned();
        let retrieved_tx3 = retrieved_tx1_map.get(&tx3.hash()).cloned();

        assert!(
            retrieved_tx1.is_none(),
            "tx1 should be removed as it was in the immutable block"
        );

        assert!(
            retrieved_tx2.is_none(),
            "tx2 should be removed as it was in the immutable block"
        );

        assert!(
            retrieved_tx3.is_some(),
            "tx3 should remain as it wasn't in the immutable block"
        );
    }
}
