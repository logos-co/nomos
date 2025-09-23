use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use futures::StreamExt as _;
use kzgrs_backend::dispersal::Index;
use nomos_core::{
    da::BlobId,
    sdp::{FinalizedBlockEvent, FinalizedBlockEventUpdate, Locator, ProviderId},
};
use nomos_utils::net::get_available_udp_port;
use rand::{Rng as _, thread_rng};
use serial_test::serial;
use tests::{
    common::da::{APP_ID, disseminate_with_metadata, wait_for_blob_onchain},
    nodes::{executor::Executor, validator::Validator},
    topology::{
        Topology, TopologyConfig,
        configs::membership::{GeneralMembershipConfig, MembershipNode, create_membership_configs},
    },
};

#[tokio::test]
#[serial]
async fn update_membership_and_disseminate() {
    let topology_config = TopologyConfig::validator_and_executor();
    let n_participants = topology_config.n_validators + topology_config.n_executors;

    let (ids, da_ports, blend_ports) = generate_test_ids_and_ports(n_participants);
    let topology =
        Topology::spawn_with_empty_membership(topology_config, &ids, &da_ports, &blend_ports).await;

    // Create a new membership with DA nodes.
    let membership_config = create_membership_configs(
        ids.iter()
            .zip(&da_ports)
            .zip(&blend_ports)
            .map(|((&id, &da_port), &blend_port)| MembershipNode {
                id,
                da_port: Some(da_port),
                blend_port: Some(blend_port),
            })
            .collect::<Vec<_>>()
            .as_slice(),
    )[0]
    .clone();
    let finalize_block_event = create_finalized_block_event(&membership_config);

    update_all_validators(&topology, &finalize_block_event).await;
    update_all_executors(&topology, &finalize_block_event).await;

    // Wait for nodes to initialise
    tokio::time::sleep(Duration::from_secs(5)).await;

    perform_dissemination_tests(&topology.executors()[0]).await;
}

fn generate_test_ids_and_ports(n_participants: usize) -> (Vec<[u8; 32]>, Vec<u16>, Vec<u16>) {
    let mut ids = vec![[0; 32]; n_participants];
    let mut da_ports = vec![];
    let mut blend_ports = vec![];

    for id in &mut ids {
        thread_rng().fill(id);
        da_ports.push(get_available_udp_port().unwrap());
        blend_ports.push(get_available_udp_port().unwrap());
    }

    (ids, da_ports, blend_ports)
}

fn create_finalized_block_event(
    membership_config: &GeneralMembershipConfig,
) -> FinalizedBlockEvent {
    let providers = membership_config
        .service_settings
        .backend
        .session_zero_providers
        .get(&nomos_core::sdp::ServiceType::DataAvailability)
        .expect("Expected data availability providers");

    let finalized_block_event_updates = create_block_event_updates(providers);

    FinalizedBlockEvent {
        block_number: 1,
        updates: finalized_block_event_updates,
    }
}

fn create_block_event_updates(
    providers: &HashMap<ProviderId, BTreeSet<Locator>>,
) -> Vec<FinalizedBlockEventUpdate> {
    providers
        .iter()
        .map(|(provider_id, locators)| FinalizedBlockEventUpdate {
            service_type: nomos_core::sdp::ServiceType::DataAvailability,
            provider_id: *provider_id,
            state: nomos_core::sdp::FinalizedDeclarationState::Active,
            locators: locators.clone(),
        })
        .collect()
}

async fn update_all_validators(topology: &Topology, finalize_block_event: &FinalizedBlockEvent) {
    for validator in topology.validators() {
        update_validator_membership(validator, finalize_block_event).await;
    }
}

async fn update_all_executors(topology: &Topology, finalize_block_event: &FinalizedBlockEvent) {
    for executor in topology.executors() {
        update_executor_membership(executor, finalize_block_event).await;
    }
}

async fn update_validator_membership(
    validator: &Validator,
    finalize_block_event: &FinalizedBlockEvent,
) {
    let res = validator
        .update_membership(finalize_block_event.clone())
        .await;
    assert!(res.is_ok(), "Failed to update membership on validator");

    for block_number in 2..=3 {
        let res = validator
            .update_membership(FinalizedBlockEvent {
                block_number,
                updates: vec![],
            })
            .await;
        assert!(res.is_ok(), "Failed to update membership on validator");
    }
}

async fn update_executor_membership(
    executor: &Executor,
    finalize_block_event: &FinalizedBlockEvent,
) {
    let res = executor
        .update_membership(finalize_block_event.clone())
        .await;
    assert!(res.is_ok(), "Failed to update membership on executor");

    for block_number in 2..=3 {
        let res = executor
            .update_membership(FinalizedBlockEvent {
                block_number,
                updates: vec![],
            })
            .await;
        assert!(res.is_ok(), "Failed to update membership on executor");
    }
}

async fn perform_dissemination_tests(executor: &Executor) {
    const ITERATIONS: usize = 10;
    let data = [1u8; 31];
    let metadata = create_test_metadata();
    let mut onchain = false;

    for i in 0..ITERATIONS {
        println!("iteration {i}");
        let blob_id = disseminate_with_metadata(executor, &data, metadata)
            .await
            .unwrap();

        if !onchain {
            wait_for_blob_onchain(executor, blob_id).await;
            onchain = true;
        }

        verify_share_replication(executor, blob_id).await;
    }
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

    assert_eq!(shares.len(), 2);
}
