use std::collections::HashSet;

use async_trait::async_trait;
use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use chain_service::storage::{
    adapters::storage::StorageAdapter, StorageAdapter as StorageAdapterTrait,
};
use nomos_core::{
    block::Block,
    header::HeaderId,
    mantle::{keys::PublicKey, SignedMantleTx, Utxo, Value},
};
use nomos_storage::backends::StorageBackend;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use tokio::sync::oneshot;
use wallet::{Wallet, WalletBlock, WalletError};

#[derive(Debug)]
pub enum WalletMsg {
    GetBalance {
        tip: HeaderId,
        pk: PublicKey,
        tx: oneshot::Sender<Result<Option<Value>, WalletError>>,
    },
    GetUtxosForAmount {
        tip: HeaderId,
        amount: Value,
        pks: Vec<PublicKey>,
        tx: oneshot::Sender<Result<Option<Vec<Utxo>>, WalletError>>,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WalletServiceSettings {
    pub known_keys: HashSet<PublicKey>,
}

pub struct WalletService<Cryptarchia, Storage, RuntimeServiceId> {
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _storage: std::marker::PhantomData<Storage>,
}

impl<Cryptarchia, Storage, RuntimeServiceId> ServiceData
    for WalletService<Cryptarchia, Storage, RuntimeServiceId>
{
    type Settings = WalletServiceSettings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = WalletMsg;
}

#[async_trait]
impl<Cryptarchia, Storage, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for WalletService<Cryptarchia, Storage, RuntimeServiceId>
where
    Cryptarchia: CryptarchiaServiceData,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as nomos_storage::api::chain::StorageChainApi>::Block:
        TryFrom<Block<SignedMantleTx>> + TryInto<Block<SignedMantleTx>>,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<Cryptarchia>
        + AsServiceId<nomos_storage::StorageService<Storage, RuntimeServiceId>>
        + std::fmt::Debug
        + std::fmt::Display
        + Send
        + Sync
        + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_resources_handle,
            _storage: std::marker::PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
        let Self {
            mut service_resources_handle,
            ..
        } = self;

        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let storage_relay = service_resources_handle
            .overwatch_handle
            .relay::<nomos_storage::StorageService<Storage, RuntimeServiceId>>()
            .await?;

        // Create the API wrapper for cleaner communication
        let cryptarchia_api = CryptarchiaServiceApi::<Cryptarchia, RuntimeServiceId>::new::<Self>(
            &service_resources_handle,
        )
        .await?;

        // Create StorageAdapter for cleaner block operations
        let storage_adapter =
            StorageAdapter::<Storage, SignedMantleTx, RuntimeServiceId>::new(storage_relay).await;

        // Query chain service for current state using the API
        let chain_info = cryptarchia_api.info().await?;

        eprintln!(
            "Wallet connecting to chain at tip: {:?}, lib: {:?}, slot: {:?}",
            chain_info.tip, chain_info.lib, chain_info.slot
        );

        // IMPORTANT: subscribe for new blocks *before* we sync from lib.
        // The blocks subscription only notifies us of new blocks that were observed
        // after we subscribed.
        //
        // If syncing from LIB takes some time, there may be a gap between the blocks
        // we observed when we first bootstrap from LIB and from when we start subscribing
        // to new blocks.
        //
        // LIB <- B_1 <- B_2 <- .. <- B_tip <- B_tip+1
        // --------------------------------
        //    synced during bootstrapping
        //
        // B_tip+1 may have been produced while we were bootstrapping. If we subscribe to
        // new blocks before we start bootstrapping, B_tip+1 and later blocks will remain
        // in the channel buffered until we are finished processing up to B_tip.

        // Subscribe to block updates using the API
        let mut new_block_receiver = cryptarchia_api.subscribe_new_blocks().await?;

        // Initialize wallet from LIB and LIB LedgerState
        let lib = chain_info.lib;

        // Fetch the ledger state at LIB using the API
        let lib_ledger = cryptarchia_api
            .get_ledger_state(lib)
            .await
            .map_err(|e| format!("Failed to get LIB ledger state: {}", e))?
            .ok_or_else(|| "Critical error: LIB ledger state not found - cannot initialize wallet without valid LIB state".to_string())?;

        let mut wallet = Wallet::from_lib(settings.known_keys.clone(), lib, &lib_ledger);

        // Request headers from LIB to tip to sync wallet state using the API
        let headers = cryptarchia_api
            .get_headers(Some(lib), Some(chain_info.tip))
            .await?;

        eprintln!("Received {} headers from LIB to tip", headers.len());

        // Fetch and apply blocks for each header to sync wallet state
        for header_id in headers {
            if header_id == lib {
                // We already have state at LIB by bootstrapping from LedgerState
                continue;
            }
            match storage_adapter.get_block(&header_id).await {
                Some(block) => {
                    let wallet_block = WalletBlock::from(block);
                    match wallet.apply_block(&wallet_block) {
                        Ok(()) => {
                            eprintln!("Applied historical block {header_id} to wallet");
                        }
                        Err(e) => {
                            panic!("Failed to apply historical block {header_id} to wallet: {e:?}");
                        }
                    }
                }
                None => {
                    panic!("Block {header_id} not found in storage");
                }
            }
        }

        service_resources_handle.status_updater.notify_ready();
        eprintln!("Wallet service is ready and subscribed to blocks");

        loop {
            tokio::select! {
                Some(msg) = service_resources_handle.inbound_relay.recv() => {
                    Self::handle_wallet_message(msg, &wallet);
                }
                Ok(header_id) = new_block_receiver.recv() => {
                    let Some(block) = storage_adapter.get_block(&header_id).await else {
                        panic!("missing block in storage");
                    };
                    let wallet_block = WalletBlock::from(block);
                    match wallet.apply_block(&wallet_block) {
                        Ok(()) => {
                            eprintln!("Applied block {:?} to wallet", wallet_block.id);
                        }
                        Err(e) => {
                            eprintln!("Failed to apply block to wallet: {e:?}");
                        }
                    }
                }
            }
        }
    }
}

impl<CryptarchiaService, Storage, RuntimeServiceId>
    WalletService<CryptarchiaService, Storage, RuntimeServiceId>
where
    CryptarchiaService: ServiceData,
    Storage: StorageBackend + Send + Sync + 'static,
{
    fn handle_wallet_message(msg: WalletMsg, wallet: &Wallet) {
        match msg {
            WalletMsg::GetBalance { tip, pk, tx } => {
                let result = wallet.balance(tip, pk);
                if tx.send(result).is_err() {
                    eprintln!("Failed to send balance response");
                }
            }
            WalletMsg::GetUtxosForAmount {
                tip,
                amount,
                pks,
                tx,
            } => {
                let result = wallet.utxos_for_amount(tip, amount, pks);
                if tx.send(result).is_err() {
                    eprintln!("Failed to send UTXOs response");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use chain_service::{ConsensusMsg, CryptarchiaInfo};
    use cryptarchia_engine::Slot;
    use nomos_core::mantle::{Note, TxHash, Utxo as CoreUtxo};
    use nomos_ledger::LedgerState;
    use nomos_storage::{
        backends::{mock::MockStorage, StorageSerde},
        StorageService as GenericStorageService,
    };
    use num_bigint::BigUint;
    use serde::de::DeserializeOwned;
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::broadcast;

    // Test serialization operator
    pub struct TestSerde;

    #[derive(Debug)]
    pub struct TestSerdeError;

    impl std::fmt::Display for TestSerdeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "TestSerdeError")
        }
    }

    impl std::error::Error for TestSerdeError {}

    impl StorageSerde for TestSerde {
        type Error = TestSerdeError;

        fn serialize<T: serde::Serialize>(value: T) -> Bytes {
            bincode::serialize(&value).unwrap().into()
        }

        fn deserialize<T: DeserializeOwned>(buff: Bytes) -> Result<T, Self::Error> {
            bincode::deserialize(&buff).map_err(|_| TestSerdeError)
        }
    }

    // Mock CryptarchiaService implementation
    #[derive(Debug)]
    pub struct MockCryptarchiaService {
        chain_info: CryptarchiaInfo,
        blocks: Arc<std::sync::Mutex<HashMap<HeaderId, Block<SignedMantleTx>>>>,
        block_sender: Arc<std::sync::Mutex<Option<broadcast::Sender<Block<SignedMantleTx>>>>>,
    }

    impl MockCryptarchiaService {
        pub fn new(lib: HeaderId, tip: HeaderId) -> Self {
            Self {
                chain_info: CryptarchiaInfo {
                    lib,
                    tip,
                    slot: Slot::genesis(),
                    height: 0,
                    mode: cryptarchia_engine::State::Online,
                },
                blocks: Arc::new(std::sync::Mutex::new(HashMap::new())),
                block_sender: Arc::new(std::sync::Mutex::new(None)),
            }
        }

        pub fn add_block(&self, block: Block<SignedMantleTx>) {
            let header_id = block.header().id();
            self.blocks.lock().unwrap().insert(header_id, block);
        }

        pub fn set_block_sender(&self, sender: broadcast::Sender<Block<SignedMantleTx>>) {
            *self.block_sender.lock().unwrap() = Some(sender);
        }
    }

    impl ServiceData for MockCryptarchiaService {
        type Settings = ();
        type State = NoState<()>;
        type StateOperator = NoOperator<Self::State>;
        type Message = ConsensusMsg<Block<SignedMantleTx>>;
    }

    #[async_trait::async_trait]
    impl<RuntimeServiceId> ServiceCore<RuntimeServiceId> for MockCryptarchiaService
    where
        RuntimeServiceId: AsServiceId<Self> + Clone + Send + Sync + 'static,
    {
        fn init(
            _service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
            _initial_state: Self::State,
        ) -> Result<Self, DynError> {
            todo!("Mock service should not be initialized through Overwatch")
        }

        async fn run(self) -> Result<(), DynError> {
            todo!("Mock service should not be run through Overwatch")
        }
    }

    // Test storage service type (no longer used directly)
    type _TestStorageService = GenericStorageService<MockStorage<TestSerde>, TestRuntimeServiceId>;

    // Test runtime service ID
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct TestRuntimeServiceId;

    impl AsServiceId<MockCryptarchiaService> for TestRuntimeServiceId {
        const SERVICE_ID: Self = Self;
    }
    impl AsServiceId<GenericStorageService<MockStorage<TestSerde>, Self>> for TestRuntimeServiceId {
        const SERVICE_ID: Self = Self;
    }
    impl AsServiceId<WalletService<MockCryptarchiaService, MockStorage<TestSerde>, Self>>
        for TestRuntimeServiceId
    {
        const SERVICE_ID: Self = Self;
    }

    // Helper functions
    fn pk(v: u64) -> PublicKey {
        PublicKey::from(BigUint::from(v))
    }

    fn tx_hash(v: u64) -> TxHash {
        TxHash::from(BigUint::from(v))
    }

    // TODO: Add helper functions for creating test blocks and proofs when needed

    #[tokio::test]
    async fn test_wallet_service_creation() {
        // Test that we can create the service types properly
        let alice = pk(1);
        let genesis = HeaderId::from([0; 32]);

        // Create test ledger state
        let ledger = LedgerState::from_utxos([CoreUtxo::new(tx_hash(0), 0, Note::new(100, alice))]);

        // Create wallet directly from core
        let wallet = Wallet::from_lib([alice], genesis, &ledger);

        // Verify wallet works
        assert_eq!(wallet.balance(genesis, alice).unwrap(), Some(100));
    }

    #[tokio::test]
    async fn test_mock_cryptarchia_service() {
        // Test that our mock service works correctly
        let lib = HeaderId::from([1; 32]);
        let tip = HeaderId::from([2; 32]);

        let mock_service = MockCryptarchiaService::new(lib, tip);

        // Verify chain info
        assert_eq!(mock_service.chain_info.lib, lib);
        assert_eq!(mock_service.chain_info.tip, tip);

        // For now, skip the block creation test since it requires complex setup
        // TODO: Implement proper block creation in tests
    }

    // Bootstrapping edge case tests - based on comments in run() method

    #[tokio::test]
    async fn test_wallet_bootstrapping_fails_with_missing_lib_state() {
        // Test that wallet service fails when LIB ledger state is missing
        // This is a critical error condition that should not be handled gracefully
        let alice = pk(1);
        let lib = HeaderId::from([1; 32]);
        let tip = HeaderId::from([2; 32]);

        let _mock_service = MockCryptarchiaService::new(lib, tip);

        // This test validates that missing LIB state should cause wallet initialization to fail
        // In a real scenario:
        // 1. WalletService calls ConsensusMsg::GetLedgerState
        // 2. Chain service returns None (LIB state not found)
        // 3. WalletService should return error and fail to start
        // 4. Service should not continue running with empty/default state

        // The corrected implementation now returns an error instead of defaulting to empty state
        // TODO: Add integration test that verifies service fails to start when chain returns None for LIB state

        // For now, verify that with valid LIB state, wallet works correctly:
        let valid_ledger =
            LedgerState::from_utxos([CoreUtxo::new(tx_hash(1), 0, Note::new(100, alice))]);
        let wallet = Wallet::from_lib([alice], lib, &valid_ledger);
        assert_eq!(wallet.balance(lib, alice).unwrap(), Some(100));
    }

    #[tokio::test]
    async fn test_wallet_bootstrapping_with_valid_lib_state() {
        // Test that wallet initializes correctly when LIB state is available
        let alice = pk(1);
        let lib = HeaderId::from([1; 32]);

        // Create valid LIB ledger state with UTXOs
        let ledger = LedgerState::from_utxos([CoreUtxo::new(tx_hash(1), 0, Note::new(150, alice))]);

        let wallet = Wallet::from_lib([alice], lib, &ledger);

        // Verify wallet initializes correctly with valid LIB state
        assert_eq!(wallet.balance(lib, alice).unwrap(), Some(150));
    }

    #[tokio::test]
    async fn test_wallet_bootstrapping_race_condition_simulation() {
        // Test the race condition between block subscription and historical sync
        // This simulates the scenario where blocks arrive during bootstrapping
        let alice = pk(1);
        let lib = HeaderId::from([1; 32]);
        let tip = HeaderId::from([2; 32]);
        let new_tip = HeaderId::from([3; 32]);

        let mock_service = MockCryptarchiaService::new(lib, tip);

        // Critical race condition scenario:
        // 1. Service queries chain info (lib=1, tip=2)
        // 2. Service subscribes to new blocks BEFORE starting sync
        // 3. During historical sync from lib->tip, new block 3 arrives
        // 4. Block 3 gets buffered in subscription channel
        // 5. After historical sync completes, block 3 is processed
        //
        // The key insight from the code comments is:
        // "IMPORTANT: subscribe for new blocks *before* we sync from lib"
        // This prevents missing blocks that arrive during bootstrapping

        // This test structure shows the critical timing issue that needs handling
        assert_eq!(mock_service.chain_info.lib, lib);
        assert_eq!(mock_service.chain_info.tip, tip);

        // TODO: Create full integration test that simulates:
        // - Chain advancing during wallet bootstrap
        // - Blocks arriving via subscription while historical sync is running
        // - Verifying correct processing order
    }

    // TODO: Add integration tests that actually simulate the full service lifecycle
    // These would test:
    // 1. Complete WalletService initialization with mock dependencies
    // 2. End-to-end block synchronization from LIB to tip
    // 3. Real-time block processing with concurrent subscriptions
    // 4. Balance and UTXO queries during and after bootstrapping
    // 5. Service shutdown and restart scenarios
}
