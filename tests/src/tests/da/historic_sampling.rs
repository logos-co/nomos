use futures::StreamExt as _;
use kzgrs_backend::dispersal::Index;
use nomos_core::da::BlobId;
use nomos_sdp::BlockEvent;
use tests::{
    common::da::{APP_ID, disseminate_with_metadata, wait_for_blob_onchain},
    nodes::executor::Executor,
    topology::{Topology, TopologyConfig},
};
use tokio::time::Duration;

#[ignore = "Reenable after transaction mempool is used"]
#[tokio::test]
async fn test_historical_sampling_across_sessions() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let executor = &topology.executors()[0];

    // Disseminate some blobs in session 0
    tokio::time::sleep(Duration::from_secs(15)).await;
    let blob_ids = disseminate_blobs_in_session_zero(executor).await;

    // Blocks 1-4: Complete session 0 and form session 1 on ALL nodes
    for block_num in 1..=4 {
        update_all_nodes(
            &topology,
            BlockEvent {
                block_number: block_num,
                updates: vec![],
            },
        )
        .await;
    }

    // todo: add more complex cases with multiple sessions

    // Wait for propagation
    tokio::time::sleep(Duration::from_secs(5)).await;

    test_sampling_scenarios(executor, &blob_ids).await;
}

async fn disseminate_blobs_in_session_zero(executor: &Executor) -> Vec<BlobId> {
    let mut blob_ids = Vec::new();
    let data = [1u8; 31];
    let metadata = create_test_metadata();

    for i in 0..3 {
        let blob_id = disseminate_with_metadata(executor, &data, metadata)
            .await
            .expect("Failed to disseminate blob");

        blob_ids.push(blob_id);

        if i == 0 {
            wait_for_blob_onchain(executor, blob_id).await;
        }

        verify_share_replication(executor, blob_id).await;
    }

    blob_ids
}

async fn update_all_nodes(topology: &Topology, event: BlockEvent) {
    // Update all validators
    for validator in topology.validators() {
        validator
            .update_membership(event.clone())
            .await
            .expect("Failed to update validator membership");
    }

    // Update all executors
    for executor in topology.executors() {
        executor
            .update_membership(event.clone())
            .await
            .expect("Failed to update executor membership");
    }
}

async fn test_sampling_scenarios(executor: &Executor, blob_ids: &[BlobId]) {
    let block_id = [0u8; 32];

    // Test 1: Valid blobs
    let valid_future = async {
        let result = executor
            .da_historic_sampling(0, block_id.into(), blob_ids.to_vec())
            .await
            .expect("HTTP request should succeed");
        assert!(
            result,
            "Historical sampling should return true for session 0 where data exists"
        );
    };

    // Test 2: Mixed valid/invalid blobs
    let invalid_future = async {
        let block_id = [1u8; 32];
        let mut mixed_blob_ids = blob_ids.to_vec();
        mixed_blob_ids[0] = [99u8; 32];

        let result = executor
            .da_historic_sampling(0, block_id.into(), mixed_blob_ids)
            .await
            .expect("HTTP request should succeed");
        assert!(
            !result,
            "Historical sampling should return false when any blob is invalid"
        );
    };

    // Run both tests concurrently
    tokio::join!(valid_future, invalid_future);
}

fn create_test_metadata() -> kzgrs_backend::dispersal::Metadata {
    let app_id = hex::decode(APP_ID).unwrap();
    let app_id: [u8; 32] = app_id.try_into().unwrap();
    kzgrs_backend::dispersal::Metadata::new(app_id, Index::from(0))
}

async fn verify_share_replication(executor: &Executor, blob_id: BlobId) {
    let shares = executor
        .get_shares(blob_id, [].into(), [].into(), true)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    assert!(!shares.is_empty(), "Should have replicated shares");
}
