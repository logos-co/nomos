use std::collections::BTreeSet;

use nomos_core::sdp::{FinalizedBlockEvent, FinalizedBlockEventUpdate, ProviderId};
use nomos_libp2p::{ed25519, Multiaddr};
use tests::{
    nodes::executor::Executor,
    secret_key_to_provider_id,
    topology::{Topology, TopologyConfig},
};

#[tokio::test]
async fn test_update_get_membership_http() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let executor = &topology.executors()[0];

    // Setup initial provider and send first update
    let provider_id = get_initial_provider_id(executor);
    let mut locators = create_initial_locators(executor);
    send_provider_update(executor, 1, *provider_id, locators.clone()).await;

    // Add second provider and send update
    let (some_provider_id, some_addr) = create_random_provider();
    locators.insert(nomos_core::sdp::Locator(some_addr));
    send_provider_update(executor, 2, some_provider_id, locators).await;

    // Test session 0 membership
    verify_session_zero_membership(executor).await;

    // Test session 1 not yet formed
    verify_session_one_not_formed(executor).await;

    // Complete the session
    complete_session(executor).await;

    // Test session 1 membership
    verify_session_one_membership(executor).await;
}

fn get_initial_provider_id(executor: &Executor) -> &ProviderId {
    executor
        .config()
        .membership
        .backend
        .session_zero_membership
        .get(&nomos_core::sdp::ServiceType::DataAvailability)
        .expect("Expected data availability membership")
        .iter()
        .next()
        .expect("Expected at least one provider ID")
}

fn create_initial_locators(executor: &Executor) -> BTreeSet<nomos_core::sdp::Locator> {
    let mut locators = BTreeSet::default();
    locators.insert(nomos_core::sdp::Locator(
        executor.config().network.backend.initial_peers[0].clone(),
    ));
    locators
}

async fn send_provider_update(
    executor: &Executor,
    block_number: u64,
    provider_id: ProviderId,
    locators: BTreeSet<nomos_core::sdp::Locator>,
) {
    executor
        .update_membership(FinalizedBlockEvent {
            block_number,
            updates: vec![FinalizedBlockEventUpdate {
                service_type: nomos_core::sdp::ServiceType::DataAvailability,
                provider_id,
                state: nomos_core::sdp::FinalizedDeclarationState::Active,
                locators,
            }],
        })
        .await
        .unwrap();
}

async fn verify_session_zero_membership(executor: &Executor) {
    let membership = executor.da_get_membership(0).await.unwrap();
    assert_eq!(membership.assignations.len(), 2);
    assert_eq!(membership.assignations.get(&0).unwrap().len(), 2);
    assert_eq!(membership.addressbook.len(), 2);
}

async fn verify_session_one_not_formed(executor: &Executor) {
    let membership = executor.da_get_membership(1).await.unwrap();
    assert_eq!(membership.assignations.len(), 0);
}

async fn complete_session(executor: &Executor) {
    for block_num in 3..=4 {
        executor
            .update_membership(FinalizedBlockEvent {
                block_number: block_num,
                updates: vec![],
            })
            .await
            .unwrap();
    }
}

async fn verify_session_one_membership(executor: &Executor) {
    let membership = executor.da_get_membership(1).await.unwrap();
    assert_eq!(membership.assignations.len(), 2);
    assert!(membership.assignations.contains_key(&0));
    assert_eq!(membership.addressbook.len(), 3);
}

fn create_random_provider() -> (ProviderId, Multiaddr) {
    let mut node_key_bytes: [u8; 32] = rand::random();
    let some_addr: Multiaddr = "/ip4/127.0.0.1/udp/10000/quic-v1".parse().unwrap();
    let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
        .expect("Failed to generate secret key from bytes");
    (secret_key_to_provider_id(node_key), some_addr)
}
