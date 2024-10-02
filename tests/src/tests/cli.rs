use nomos_cli::cmds::disseminate::Disseminate;
use nomos_cli::da::network::backend::ExecutorBackend;
use nomos_cli::da::network::backend::ExecutorBackendSettings;
use nomos_da_network_service::NetworkConfig;
use nomos_libp2p::ed25519;
use nomos_libp2p::libp2p;
use nomos_libp2p::Multiaddr;
use nomos_libp2p::PeerId;
use std::collections::HashMap;
use std::time::Duration;
use subnetworks_assignations::versions::v1::FillFromNodeList;
use tempfile::NamedTempFile;
use tests::nodes::NomosNode;
use tests::Node;
use tests::SpawnConfig;
use tests::GLOBAL_PARAMS_PATH;

const CLI_BIN: &str = "../target/debug/nomos-cli";
const APP_ID: &str = "fd3384e132ad02a56c78f45547ee40038dc79002b90d29ed90e08eee762ae715";

use std::process::Command;

fn run_disseminate(disseminate: &Disseminate) {
    let mut binding = Command::new(CLI_BIN);
    let c = binding
        .args(["disseminate", "--network-config"])
        .arg(disseminate.network_config.as_os_str())
        .arg("--app-id")
        .arg(&disseminate.app_id)
        .arg("--index")
        .arg(disseminate.index.to_string())
        .arg("--columns")
        .arg(disseminate.columns.to_string())
        .arg("--timeout")
        .arg(disseminate.timeout.to_string())
        .arg("--wait-until-disseminated")
        .arg(disseminate.wait_until_disseminated.to_string())
        .arg("--node-addr")
        .arg(disseminate.node_addr.as_ref().unwrap().as_str())
        .arg("--global-params-path")
        .arg(GLOBAL_PARAMS_PATH.to_string());

    match (&disseminate.data, &disseminate.file) {
        (Some(data), None) => c.args(["--data", &data]),
        (None, Some(file)) => c.args(["--file", file.as_os_str().to_str().unwrap()]),
        (_, _) => panic!("Either data or file needs to be provided, but not both"),
    };

    c.status().expect("failed to execute nomos cli");
}

async fn disseminate(nodes: &Vec<NomosNode>, config: &mut Disseminate) {
    // Nomos Cli is acting as the first node when dispersing the data by using the key associated
    // with that Nomos Node.
    let first_config = nodes[0].config();
    let node_key = first_config.da_network.backend.node_key.clone();
    let node_addrs: HashMap<PeerId, Multiaddr> = nodes
        .iter()
        .map(|n| {
            let libp2p_config = &n.config().network.backend.inner;
            let keypair = libp2p::identity::Keypair::from(ed25519::Keypair::from(
                libp2p_config.node_key.clone(),
            ));
            let peer_id = PeerId::from(keypair.public());
            let address = n
                .config()
                .da_network
                .backend
                .listening_address
                .clone()
                .with_p2p(peer_id)
                .unwrap();
            (peer_id, address)
        })
        .collect();
    let membership = first_config.da_network.backend.membership.clone();
    let num_subnets = first_config.da_sampling.sampling_settings.num_subnets;

    let da_network_config: NetworkConfig<ExecutorBackend<FillFromNodeList>> = NetworkConfig {
        backend: ExecutorBackendSettings {
            node_key,
            membership,
            node_addrs,
            num_subnets,
        },
    };

    let mut file = NamedTempFile::new().unwrap();
    let config_path = file.path().to_owned();
    serde_yaml::to_writer(&mut file, &da_network_config).unwrap();

    config.network_config = config_path;
    config.node_addr = Some(
        format!(
            "http://{}",
            nodes[0].config().http.backend_settings.address.clone()
        )
        .parse()
        .unwrap(),
    );

    run_disseminate(&config);
}

#[tokio::test]
async fn disseminate_and_retrieve() {
    let mut config = Disseminate {
        data: Some("hello world".to_string()),
        timeout: 60,
        wait_until_disseminated: 5,
        app_id: APP_ID.into(),
        index: 0,
        columns: 2,
        ..Default::default()
    };

    let nodes = NomosNode::spawn_nodes(SpawnConfig::star_happy(
        2,
        tests::DaConfig {
            dispersal_factor: 2,
            subnetwork_size: 2,
            num_subnets: 2,
            ..Default::default()
        },
    ))
    .await;

    disseminate(&nodes, &mut config).await;
    tokio::time::sleep(Duration::from_secs(10)).await;

    let from = 0u64.to_be_bytes();
    let to = 1u64.to_be_bytes();
    let app_id = hex::decode(APP_ID).unwrap();

    let node1_blobs = nodes[0]
        .get_indexer_range(app_id.clone().try_into().unwrap(), from..to)
        .await;
    let node2_blobs = nodes[1]
        .get_indexer_range(app_id.try_into().unwrap(), from..to)
        .await;

    let node1_idx_0_blobs: Vec<_> = node1_blobs
        .iter()
        .filter(|(i, _)| i == &from)
        .flat_map(|(_, blobs)| blobs)
        .collect();
    let node2_idx_0_blobs: Vec<_> = node2_blobs
        .iter()
        .filter(|(i, _)| i == &from)
        .flat_map(|(_, blobs)| blobs)
        .collect();

    // Index zero shouldn't be empty, node 2 replicated both blobs to node 1 because they both
    // are in the same subnetwork.
    for b in node1_idx_0_blobs.iter() {
        assert!(!b.is_empty())
    }

    for b in node2_idx_0_blobs.iter() {
        assert!(!b.is_empty())
    }
}
