use futures::StreamExt as _;
use kzgrs_backend::dispersal::Index;
use nomos_core::sdp::{FinalizedBlockEvent, FinalizedBlockEventUpdate};
use rand::{thread_rng, Rng as _};
use tests::{
    common::da::{disseminate_with_metadata, wait_for_blob_onchain, APP_ID},
    get_available_port,
    topology::{configs::membership::create_membership_configs, Topology, TopologyConfig},
};

#[tokio::test]
async fn update_membership_and_dissiminate() {
    const ITERATIONS: usize = 10;
    let topology_config = TopologyConfig::validator_and_executor();
    let n_participants = topology_config.n_validators + topology_config.n_executors;

    // we use the same random bytes for:
    // * da id
    // * coin sk
    // * coin nonce
    // * libp2p node key
    let mut ids = vec![[0; 32]; n_participants];
    let mut ports = vec![];
    for id in &mut ids {
        thread_rng().fill(id);
        ports.push(get_available_port());
    }

    let topology = Topology::spawn_with_empty_membership(topology_config, &ids, &ports).await;

    let non_empty_membership_configs = create_membership_configs(&ids, &ports)[0].clone();

    let non_zero_membership = non_empty_membership_configs
        .service_settings
        .backend
        .session_zero_membership
        .get(&nomos_core::sdp::ServiceType::DataAvailability)
        .expect("Expected data availability membership");

    let mut finalized_block_event_updates = vec![];

    for provider in non_zero_membership {
        let locators = non_empty_membership_configs
            .service_settings
            .backend
            .session_zero_locators_mapping
            .get(&nomos_core::sdp::ServiceType::DataAvailability)
            .expect("Expected data availability service type")
            .get(provider)
            .expect("Expected locators for provider")
            .clone();

        finalized_block_event_updates.push(FinalizedBlockEventUpdate {
            service_type: nomos_core::sdp::ServiceType::DataAvailability,
            provider_id: *provider,
            state: nomos_core::sdp::FinalizedDeclarationState::Active,
            locators,
        });
    }

    let finalize_block_event = FinalizedBlockEvent {
        block_number: 1,
        updates: finalized_block_event_updates.clone(),
    };

    for validator in topology.validators() {
        let res = validator
            .update_membership(finalize_block_event.clone())
            .await;
        assert!(res.is_ok(), "Failed to update membership on validator");
    }

    for executor in topology.executors() {
        let res = executor
            .update_membership(finalize_block_event.clone())
            .await;
        assert!(res.is_ok(), "Failed to update membership on executor");
    }

    let executor = &topology.executors()[0];
    let data = [1u8; 31];

    let app_id = hex::decode(APP_ID).unwrap();
    let app_id: [u8; 32] = app_id.clone().try_into().unwrap();
    let metadata = kzgrs_backend::dispersal::Metadata::new(app_id, Index::from(0));

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

        let shares = executor
            .get_shares(blob_id, [].into(), [].into(), true)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;

        // Validator replicated both blobs to
        // executor because they both are in the same subnetwork.
        assert!(shares.len() == 2);
    }
}
