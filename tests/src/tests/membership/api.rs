use std::collections::BTreeSet;

use nomos_core::sdp::{FinalizedBlockEvent, FinalizedBlockEventUpdate};
use tests::topology::{Topology, TopologyConfig};

#[tokio::test]
async fn test_update_membership_http() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let executor = &topology.executors()[0];
    let provider_id = executor
        .config()
        .membership
        .backend
        .initial_membership
        .get(&0)
        .expect("Expected at least one membership entry")
        .get(&nomos_core::sdp::ServiceType::DataAvailability)
        .expect("Expected at least one provider ID in the membership set")
        .iter()
        .next()
        .expect("Expected at least one provider ID in the membership set");

    let mut locators = BTreeSet::default();
    locators.insert(nomos_core::sdp::Locator(
        executor.config().network.backend.initial_peers[0].clone(),
    ));

    let res = executor
        .update_membership(FinalizedBlockEvent {
            block_number: 1,
            updates: vec![FinalizedBlockEventUpdate {
                service_type: nomos_core::sdp::ServiceType::DataAvailability,
                provider_id: *provider_id,
                state: nomos_core::sdp::DeclarationState::Active,
                locators,
            }],
        })
        .await;

    res.unwrap();
}
