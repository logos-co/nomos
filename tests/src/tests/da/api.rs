use std::collections::HashSet;

use common_http_client::CommonHttpClient;
use futures_util::stream::StreamExt as _;
use kzgrs_backend::common::share::{DaLightShare, DaShare};
use nomos_core::da::blob::{LightShare as _, Share as _};
use nomos_libp2p::ed25519;
use rand::{rngs::OsRng, RngCore as _};
use reqwest::Url;
use subnetworks_assignations::MembershipHandler as _;
use tests::{
    common::da::{disseminate_with_metadata, wait_for_indexed_blob, APP_ID},
    secret_key_to_peer_id,
    topology::{Topology, TopologyConfig},
};

#[tokio::test]
async fn test_get_share_data() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let executor = &topology.executors()[0];
    let num_subnets = executor.config().da_network.backend.num_subnets as usize;

    let data = [1u8; 31];
    let app_id = hex::decode(APP_ID).unwrap();
    let app_id: [u8; 32] = app_id.clone().try_into().unwrap();
    let metadata = kzgrs_backend::dispersal::Metadata::new(app_id, 0u64.into());

    disseminate_with_metadata(executor, &data, metadata).await;

    let from = 0u64.to_be_bytes();
    let to = 1u64.to_be_bytes();

    wait_for_indexed_blob(executor, app_id, from, to, num_subnets).await;

    let executor_blobs = executor.get_indexer_range(app_id, from..to).await;

    let share = executor_blobs
        .iter()
        .flat_map(|(_, shares)| shares)
        .next()
        .unwrap();

    let exec_url =
        Url::parse(format!("http://{}", executor.config().http.backend_settings.address).as_str())
            .unwrap();

    let client = CommonHttpClient::new(None);
    let commitments = client
        .get_commitments::<DaShare>(exec_url.clone(), share.blob_id().try_into().unwrap())
        .await
        .unwrap();

    assert!(commitments.is_some());

    let share_data = client
        .get_share::<DaShare, DaLightShare>(
            exec_url,
            share.blob_id().try_into().unwrap(),
            share.share_idx(),
        )
        .await
        .unwrap();

    assert!(share_data.is_some());
}

#[tokio::test]
async fn test_block_peer() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let executor = &topology.executors()[0];

    let blacklisted_peers = executor.blacklisted_peers().await;
    assert!(blacklisted_peers.is_empty());

    let membership = executor.config().da_network.membership.members();

    // take second peer ID from the membership set
    let existing_peer_id = membership
        .iter()
        .nth(1)
        .expect("Expected at least 2 members in the set");

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

    let data = [1u8; 31];
    let app_id = hex::decode(APP_ID).unwrap();
    let app_id: [u8; 32] = app_id.try_into().unwrap();
    let metadata = kzgrs_backend::dispersal::Metadata::new(app_id, 0u64.into());

    disseminate_with_metadata(executor, &data, metadata).await;
    let from = 0u64.to_be_bytes();
    let to = 1u64.to_be_bytes();
    wait_for_indexed_blob(executor, app_id, from, to, num_subnets).await;

    let executor_blobs = executor.get_indexer_range(app_id, from..to).await;
    let blob = executor_blobs
        .iter()
        .flat_map(|(_, blobs)| blobs)
        .next()
        .unwrap();
    let blob_id = blob.blob_id().try_into().unwrap();

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

    // Test case 3: Request only the first share but return all available shares
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
