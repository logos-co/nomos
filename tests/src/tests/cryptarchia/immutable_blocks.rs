use std::{num::NonZero, time::Duration};

use futures_util::StreamExt as _;
use nomos_node::config::cryptarchia::deployment::Settings as CryptarchiaDeploymentSettings;
use serial_test::serial;
use tests::{
    adjust_timeout,
    nodes::validator::{Validator, create_validator_config},
    topology::configs::create_general_configs,
};

const IMMUTABLE_BLOCK_COUNT: u64 = 5;
const TEST_DURATION_SECS: u64 = 120;

#[tokio::test]
#[serial]
async fn immutable_blocks_two_nodes() {
    let configs = create_general_configs(2)
        .into_iter()
        .map(|mut c| {
            c.time_config.slot_duration = Duration::from_secs(3);
            c.consensus_config
                .user_config
                .service
                .bootstrap
                .prolonged_bootstrap_period = Duration::ZERO;
            let mut config = create_validator_config(c);
            // TODO: Find a different way to access the deployment settings, whatever input
            // they were given. I.e., should be able to access each service property after
            // it has been evaluated as a well-known deployment or a custom one. This means
            // distinguishing between the serializable type and the actual type used. Then,
            // on the actual type, we can modify stuff from CLI and env, and use that in the
            // tests instead.
            config
                .cryptarchia
                .c
                .consensus_config
                .ledger_config
                .consensus_config
                .security_param = NonZero::new(5).unwrap();
            config.time.backend_settings.slot_config.slot_duration = Duration::from_secs(3);
            config
        })
        .collect::<Vec<_>>();

    let nodes = futures_util::future::join_all(configs.into_iter().map(Validator::spawn))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let [node1, node2] = &nodes[..] else {
        panic!("Incorrect number of validators");
    };

    let (stream1, stream2) = (
        node1.get_lib_stream().await.unwrap(),
        node2.get_lib_stream().await.unwrap(),
    );

    tokio::pin!(stream1);
    tokio::pin!(stream2);

    let timeout = tokio::time::sleep(adjust_timeout(Duration::from_secs(TEST_DURATION_SECS)));

    tokio::select! {
        () = timeout => panic!("Timed out waiting for matching LIBs"),
        () = async {
            let mut stream = stream1.zip(stream2);

            while let Some((lib1, lib2)) = stream.next().await {
                println!("Node 1 LIB: height={}, id={}", lib1.height, lib1.header_id);
                println!("Node 2 LIB: height={}, id={}", lib2.height, lib2.header_id);

                assert!(!(lib1 != lib2),
                    "LIBs mismatched! Node 1: {lib1:?}, Node 2: {lib2:?}");

                if lib1.height >= IMMUTABLE_BLOCK_COUNT { return; }
            }

            panic!("LIB stream failed");
        } => {}
    }
}
