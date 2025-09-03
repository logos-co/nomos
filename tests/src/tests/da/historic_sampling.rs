use std::collections::BTreeSet;

use futures::StreamExt as _;
use kzgrs_backend::dispersal::Index;
use nomos_core::{
    da::BlobId,
    sdp::{FinalizedBlockEvent, FinalizedBlockEventUpdate},
};
use tests::{
    common::da::{disseminate_with_metadata, wait_for_blob_onchain, APP_ID},
    nodes::executor::Executor,
    topology::{Topology, TopologyConfig},
};

#[tokio::test]
async fn test_historical_sampling_across_sessions() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let executor = &topology.executors()[0];

    // Disseminate some blobs in session 0
    let blob_ids = disseminate_blobs_in_session_zero(executor).await;

    // Create session 1 with deactivated members
    // Block 1: Deactivate all providers on ALL nodes
    deactivate_all_providers(&topology, 1).await;

    // Blocks 2-4: Complete session 0 and form session 1 on ALL nodes
    for block_num in 2..=4 {
        update_all_nodes(
            &topology,
            FinalizedBlockEvent {
                block_number: block_num,
                updates: vec![],
            },
        )
        .await;
    }

    // Wait for propagation
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

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

async fn deactivate_all_providers(topology: &Topology, block_number: u64) {
    // Get providers from one of the executors (they all have the same config)
    let providers = topology.executors()[0]
        .config()
        .membership
        .backend
        .session_zero_membership
        .get(&nomos_core::sdp::ServiceType::DataAvailability)
        .expect("Expected data availability membership");

    let updates: Vec<FinalizedBlockEventUpdate> = providers
        .iter()
        .map(|provider_id| FinalizedBlockEventUpdate {
            service_type: nomos_core::sdp::ServiceType::DataAvailability,
            provider_id: *provider_id,
            state: nomos_core::sdp::FinalizedDeclarationState::Inactive,
            locators: BTreeSet::default(),
        })
        .collect();

    let event = FinalizedBlockEvent {
        block_number,
        updates,
    };

    update_all_nodes(topology, event).await;
}

async fn update_all_nodes(topology: &Topology, event: FinalizedBlockEvent) {
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

    // Test 1: Sampling from session 1 (deactivated members) - should fail
    let result = executor
        .da_historic_sampling(1, block_id.into(), blob_ids.to_vec())
        .await;
    assert!(result.is_err(), result.unwrap_err().to_string());

    // Test 2: Sampling from session 0 (where data was disseminated) - should
    // succeed
    let result = executor
        .da_historic_sampling(0, block_id.into(), blob_ids.to_vec())
        .await;
    assert!(result.is_ok(), result.unwrap_err().to_string());
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
