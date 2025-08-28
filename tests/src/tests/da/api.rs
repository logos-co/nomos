use std::collections::HashSet;

use common_http_client::CommonHttpClient;
use futures_util::stream::StreamExt as _;
use kzgrs_backend::common::share::DaShare;
use nomos_core::da::blob::LightShare as _;
use nomos_da_network_service::membership::adapters::service::peer_id_from_provider_id;
use nomos_libp2p::ed25519;
use rand::{rngs::OsRng, RngCore as _};
use reqwest::Url;
use tests::{
    common::da::{disseminate_with_metadata, wait_for_blob_onchain, APP_ID},
    secret_key_to_peer_id,
    topology::{Topology, TopologyConfig},
};

#[tokio::test]
async fn test_get_share_data() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let executor = &topology.executors()[0];

    let data = [1u8; 31];
    let app_id = hex::decode(APP_ID).unwrap();
    let app_id: [u8; 32] = app_id.clone().try_into().unwrap();
    let metadata = kzgrs_backend::dispersal::Metadata::new(app_id, 0u64.into());

    let blob_id = disseminate_with_metadata(executor, &data, metadata)
        .await
        .unwrap();

    wait_for_blob_onchain(executor, blob_id).await;

    let executor_shares = executor
        .get_shares(blob_id, HashSet::new(), HashSet::new(), true)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    assert!(executor_shares.len() == 2);
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

    let data = [1u8; 31];
    let app_id = hex::decode(APP_ID).unwrap();
    let app_id: [u8; 32] = app_id.try_into().unwrap();
    let metadata = kzgrs_backend::dispersal::Metadata::new(app_id, 0u64.into());

    let blob_id = disseminate_with_metadata(executor, &data, metadata)
        .await
        .unwrap();

    wait_for_blob_onchain(executor, blob_id).await;

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
