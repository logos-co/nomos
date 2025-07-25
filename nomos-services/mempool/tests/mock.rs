use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use nomos_core::{
    header::HeaderId,
    mantle::mock::{MockTransaction, MockTxId},
};
use nomos_mempool::{
    backend::mockpool::MockPool,
    network::{
        adapters::mock::{MockAdapter, MOCK_TX_CONTENT_TOPIC},
        NetworkAdapter,
    },
    processor::noop::NoOpPayloadProcessor,
    tx::{service::GenericTxMempoolService, state::TxMempoolState},
    MempoolMsg, TxMempoolSettings,
};
use nomos_network::{
    backends::mock::{Mock, MockBackendMessage, MockConfig, MockMessage},
    config::NetworkConfig,
    message::NetworkMsg,
    NetworkService,
};
use nomos_tracing_service::{Tracing, TracingSettings};
use nomos_utils::noop_service::NoService;
use overwatch::overwatch::OverwatchRunner;
use overwatch_derive::*;
use rand::distributions::{Alphanumeric, DistString as _};
use services_utils::{
    overwatch::{recovery::operators::RecoveryBackend as _, JsonFileBackend},
    traits::FromSettings as _,
};

type NoProcessor<NetworkAdapter> = NoOpPayloadProcessor<NoService, NetworkAdapter>;

type MockRecoveryBackend = JsonFileBackend<
    TxMempoolState<MockPool<HeaderId, MockTransaction<MockMessage>, MockTxId>, (), (), ()>,
    TxMempoolSettings<(), (), ()>,
>;
type MockMempoolService = GenericTxMempoolService<
    MockPool<HeaderId, MockTransaction<MockMessage>, MockTxId>,
    MockAdapter<RuntimeServiceId>,
    NoProcessor<<MockAdapter<RuntimeServiceId> as NetworkAdapter<RuntimeServiceId>>::Payload>,
    MockRecoveryBackend,
    RuntimeServiceId,
>;

#[derive_services]
struct MockPoolNode {
    logging: Tracing<RuntimeServiceId>,
    network: NetworkService<Mock, RuntimeServiceId>,
    mockpool: MockMempoolService,
    no_service: NoService,
}

fn run_with_recovery_teardown(recovery_path: &Path, run: impl Fn()) {
    run();
    let _ = std::fs::remove_file(recovery_path);
}

fn get_test_random_path() -> PathBuf {
    PathBuf::from(Alphanumeric.sample_string(&mut rand::thread_rng(), 5)).with_extension(".json")
}

#[test]
#[expect(clippy::too_many_lines, reason = "self contained test")]
fn test_mock_mempool() {
    let recovery_file_path = get_test_random_path();
    run_with_recovery_teardown(&recovery_file_path, || {
        let exist = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let exist2 = Arc::clone(&exist);

        let predefined_messages = vec![
            MockMessage {
                payload: "This is foo".to_owned(),
                content_topic: MOCK_TX_CONTENT_TOPIC,
                version: 0,
                timestamp: 0,
            },
            MockMessage {
                payload: "This is bar".to_owned(),
                content_topic: MOCK_TX_CONTENT_TOPIC,
                version: 0,
                timestamp: 0,
            },
        ];

        let exp_txns: HashSet<MockMessage> = predefined_messages.iter().cloned().collect();

        let settings = MockPoolNodeServiceSettings {
            network: NetworkConfig {
                backend: MockConfig {
                    predefined_messages,
                    duration: tokio::time::Duration::from_millis(100),
                    seed: 0,
                    version: 1,
                    weights: None,
                },
            },
            mockpool: TxMempoolSettings {
                pool: (),
                network_adapter: (),
                processor: (),
                recovery_path: recovery_file_path.clone(),
            },
            logging: TracingSettings::default(),
            no_service: (),
        };
        let app = OverwatchRunner::<MockPoolNode>::run(settings, None)
            .map_err(|e| eprintln!("Error encountered: {e}"))
            .unwrap();
        let overwatch_handle = app.handle().clone();
        let _ = app
            .runtime()
            .handle()
            .block_on(app.handle().start_all_services());

        app.spawn(async move {
            let network_outbound = overwatch_handle
                .relay::<NetworkService<_, _>>()
                .await
                .unwrap();
            let mempool_outbound = overwatch_handle
                .relay::<MockMempoolService>()
                .await
                .unwrap();

            // subscribe to the mock content topic
            network_outbound
                .send(NetworkMsg::Process(MockBackendMessage::RelaySubscribe {
                    topic: MOCK_TX_CONTENT_TOPIC.content_topic_name.to_string(),
                }))
                .await
                .unwrap();

            // try to wait all ops to be stored in mempool
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                let (mtx, mrx) = tokio::sync::oneshot::channel();
                mempool_outbound
                    .send(MempoolMsg::View {
                        ancestor_hint: [0; 32].into(),
                        reply_channel: mtx,
                    })
                    .await
                    .unwrap();

                let items: HashSet<MockMessage> = mrx
                    .await
                    .unwrap()
                    .map(|msg| msg.message().clone())
                    .collect();

                if items.len() == exp_txns.len() {
                    assert_eq!(exp_txns, items);
                    exist.store(true, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
            }
        });

        while !exist2.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        let recovery_backend = MockRecoveryBackend::from_settings(&TxMempoolSettings {
            pool: (),
            network_adapter: (),
            processor: (),
            recovery_path: recovery_file_path.clone(),
        });
        let recovered_state = recovery_backend
            .load_state()
            .expect("Should not fail to load the state.");
        assert_eq!(recovered_state.pool().unwrap().pending_items().len(), 2);
        assert_eq!(recovered_state.pool().unwrap().in_block_items().len(), 0);
        assert!(recovered_state.pool().unwrap().last_item_timestamp() > 0);

        let _ = app.runtime().handle().block_on(app.handle().shutdown());
        app.blocking_wait_finished();
    });
}
