use std::collections::BTreeSet;

use nomos_core::sdp::{FinalizedBlockEvent, FinalizedBlockEventUpdate};
use nomos_libp2p::{ed25519, Multiaddr};
use tests::{
    secret_key_to_provider_id,
    topology::{Topology, TopologyConfig},
};

#[tokio::test]
async fn test_update_get_membership_http() {
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

    executor
        .update_membership(FinalizedBlockEvent {
            block_number: 1,
            updates: vec![FinalizedBlockEventUpdate {
                service_type: nomos_core::sdp::ServiceType::DataAvailability,
                provider_id: *provider_id,
                state: nomos_core::sdp::FinalizedDeclarationState::Active,
                locators: locators.clone(),
            }],
        })
        .await
        .unwrap();

    // we add one more provider to the membership in block 2
    let mut node_key_bytes: [u8; 32] = rand::random();
    let some_addr: Multiaddr = "/ip4/127.0.0.1/udp/10000/quic-v1".parse().unwrap();
    let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
        .expect("Failed to generate secret key from bytes");

    let some_provider_id = secret_key_to_provider_id(node_key);

    locators.insert(nomos_core::sdp::Locator(some_addr));

    executor
        .update_membership(FinalizedBlockEvent {
            block_number: 2,
            updates: vec![FinalizedBlockEventUpdate {
                service_type: nomos_core::sdp::ServiceType::DataAvailability,
                provider_id: some_provider_id,
                state: nomos_core::sdp::FinalizedDeclarationState::Active,
                locators: locators.clone(),
            }],
        })
        .await
        .unwrap();

    // The first membership (block 1) is from config initial peers and has 2
    // subnetworks and 2 peerids in addressbook.
    let membership = executor.da_get_membership(1).await.unwrap();
    assert_eq!(membership.assignations.len(), 2);
    assert_eq!(membership.assignations.get(&0).unwrap().len(), 2);
    assert_eq!(membership.addressbook.len(), 2);

    // The second membership (block 2) is from the update and has 2 subnetworks and
    // 3 peerids in addressbook.
    let membership = executor.da_get_membership(2).await.unwrap();
    assert_eq!(membership.assignations.len(), 2);
    assert!(membership.assignations.contains_key(&0));
    assert_eq!(membership.addressbook.len(), 3);
}
