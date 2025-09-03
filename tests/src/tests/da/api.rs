use std::{collections::HashSet, time::Duration};

use common_http_client::CommonHttpClient;
use futures_util::stream::StreamExt as _;
use kzgrs_backend::common::share::DaShare;
use nomos_core::da::blob::LightShare as _;
use nomos_da_network_service::membership::adapters::service::peer_id_from_provider_id;
use nomos_libp2p::ed25519;
use rand::{rngs::OsRng, RngCore as _};
use reqwest::Url;
use tests::{
    adjust_timeout,
    common::da::{disseminate_with_metadata, wait_for_blob_onchain, APP_ID, DA_TESTS_TIMEOUT},
    nodes::validator::{create_validator_config, Validator},
    secret_key_to_peer_id,
    topology::{configs::create_general_configs, Topology, TopologyConfig},
};

#[tokio::test]
async fn test_get_share_data() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let executor = &topology.executors()[0];

    // Wait for nodes to initialise
    tokio::time::sleep(Duration::from_secs(5)).await;

    let data = [1u8; 31];
    let app_id = hex::decode(APP_ID).unwrap();
    let app_id: [u8; 32] = app_id.clone().try_into().unwrap();
    let metadata = kzgrs_backend::dispersal::Metadata::new(app_id, 0u64.into());

    let blob_id = disseminate_with_metadata(executor, &data, metadata)
        .await
        .unwrap();

    wait_for_blob_onchain(executor, blob_id).await;

    // Wait for transactions to be stored
    tokio::time::sleep(Duration::from_secs(2)).await;

    let executor_shares = executor
        .get_shares(blob_id, HashSet::new(), HashSet::new(), true)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    assert!(executor_shares.len() == 2);
}

#[tokio::test]
async fn test_get_commitments_from_peers() {
    let interconnected_topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let validator = &interconnected_topology.validators()[0];
    let executor = &interconnected_topology.executors()[0];

    // Wait for nodes to initialise
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Create independent node that only knows about membership of
    // `interconnected_topology` nodes. This validator will not receive any data
    // from the previous two, so it will need to query the DA network over the
    // sampling protocol for the share commitments.
    let lone_general_config = create_general_configs(1).into_iter().next().unwrap();
    let mut lone_validator_config = create_validator_config(lone_general_config);
    lone_validator_config.membership = validator.config().membership.clone();
    let lone_validator = Validator::spawn(lone_validator_config).await.unwrap();

    let data = [1u8; 31];
    let app_id = hex::decode(APP_ID).unwrap();
    let app_id: [u8; 32] = app_id.clone().try_into().unwrap();
    let metadata = kzgrs_backend::dispersal::Metadata::new(app_id, 0u64.into());

    let blob_id = disseminate_with_metadata(executor, &data, metadata)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(5)).await;
    lone_validator.get_commitments(blob_id).await.unwrap();

    let timeout = adjust_timeout(Duration::from_secs(DA_TESTS_TIMEOUT));
    assert!(
        (tokio::time::timeout(timeout, async {
            lone_validator.get_commitments(blob_id).await
        })
        .await)
            .is_ok(),
        "timed out waiting for share commitments"
    );
}

#[tokio::test]
async fn test_block_peer() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let executor = &topology.executors()[0];

    let blacklisted_peers = executor.blacklisted_peers().await;
    assert!(blacklisted_peers.is_empty());

    let membership = executor
        .config()
        .membership
        .backend
        .session_zero_membership
        .get(&nomos_core::sdp::ServiceType::DataAvailability)
        .expect("Expected data availability membership");
    assert!(!membership.is_empty());

    // take second peer ID from the membership set
    let existing_provider_id = *membership
        .iter()
        .nth(1)
        .expect("Expected at least two provider IDs in the membership set");

    let existing_peer_id = peer_id_from_provider_id(&existing_provider_id.0)
        .expect("Failed to convert provider ID to PeerId");

    // try block/unblock peer id combinations
    let blocked = executor.block_peer(existing_peer_id.to_string()).await;
    assert!(blocked);

    let blacklisted_peers = executor.blacklisted_peers().await;
    assert_eq!(blacklisted_peers.len(), 1);
    assert_eq!(blacklisted_peers[0], existing_peer_id.to_string());

    let blocked = executor.block_peer(existing_peer_id.to_string()).await;
    assert!(!blocked);

    let unblocked = executor.unblock_peer(existing_peer_id.to_string()).await;
    assert!(unblocked);

    let blacklisted_peers = executor.blacklisted_peers().await;
    assert!(blacklisted_peers.is_empty());

    let unblocked = executor.unblock_peer(existing_peer_id.to_string()).await;
    assert!(!unblocked);

    // try blocking/unblocking non existing peer id
    let mut node_key_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut node_key_bytes);

    let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
        .expect("Failed to generate secret key from bytes");

    let non_existing_peer_id = secret_key_to_peer_id(node_key);
    let blocked = executor.block_peer(non_existing_peer_id.to_string()).await;
    assert!(blocked);

    let unblocked = executor
        .unblock_peer(non_existing_peer_id.to_string())
        .await;
    assert!(unblocked);
}

#[tokio::test]
async fn test_get_shares() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let executor = &topology.executors()[0];
    let num_subnets = executor.config().da_network.backend.num_subnets as usize;

    // Wait for nodes to initialise
    tokio::time::sleep(Duration::from_secs(5)).await;

    let data = [1u8; 31];
    let app_id = hex::decode(APP_ID).unwrap();
    let app_id: [u8; 32] = app_id.try_into().unwrap();
    let metadata = kzgrs_backend::dispersal::Metadata::new(app_id, 0u64.into());

    let blob_id = disseminate_with_metadata(executor, &data, metadata)
        .await
        .unwrap();

    wait_for_blob_onchain(executor, blob_id).await;

    // Wait for transactions to be stored
    tokio::time::sleep(Duration::from_secs(2)).await;

    let exec_url = Url::parse(&format!(
        "http://{}",
        executor.config().http.backend_settings.address
    ))
    .unwrap();
    let client = CommonHttpClient::new(None);

    // Test case 1: Request all shares
    let shares_stream = client
        .get_shares::<DaShare>(
            exec_url.clone(),
            blob_id,
            HashSet::new(),
            HashSet::new(),
            true,
        )
        .await
        .unwrap();
    let shares = shares_stream.collect::<Vec<_>>().await;
    assert_eq!(shares.len(), num_subnets);
    assert!(shares.iter().any(|share| share.share_idx() == [0, 0]));
    assert!(shares.iter().any(|share| share.share_idx() == [0, 1]));

    // Test case 2: Request only the first share
    let shares_stream = client
        .get_shares::<DaShare>(
            exec_url.clone(),
            blob_id,
            HashSet::from([[0, 0]]),
            HashSet::new(),
            false,
        )
        .await
        .unwrap();
    let shares = shares_stream.collect::<Vec<_>>().await;
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0].share_idx(), [0, 0]);

    // Test case 3: Request only the first share but return all available
    // shares
    let shares_stream = client
        .get_shares::<DaShare>(
            exec_url.clone(),
            blob_id,
            HashSet::from([[0, 0]]),
            HashSet::new(),
            true,
        )
        .await
        .unwrap();
    let shares = shares_stream.collect::<Vec<_>>().await;
    assert_eq!(shares.len(), num_subnets);
    assert!(shares.iter().any(|share| share.share_idx() == [0, 0]));
    assert!(shares.iter().any(|share| share.share_idx() == [0, 1]));

    // Test case 4: Request all shares and filter out the second share
    let shares_stream = client
        .get_shares::<DaShare>(
            exec_url.clone(),
            blob_id,
            HashSet::new(),
            HashSet::from([[0, 1]]),
            true,
        )
        .await
        .unwrap();
    let shares = shares_stream.collect::<Vec<_>>().await;
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0].share_idx(), [0, 0]);

    // Test case 5: Request unavailable shares
    let shares_stream = client
        .get_shares::<DaShare>(
            exec_url.clone(),
            blob_id,
            HashSet::from([[0, 2]]),
            HashSet::new(),
            false,
        )
        .await
        .unwrap();

    let shares = shares_stream.collect::<Vec<_>>().await;
    assert!(shares.is_empty());
}
