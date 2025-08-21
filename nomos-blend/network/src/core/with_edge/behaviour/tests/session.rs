use crate::core::{tests::utils::TestSwarm, with_edge::behaviour::tests::utils::BehaviourBuilder};

#[test(tokio::test)]
async fn new_sesion() {
    let mut core1 = TestSwarm::new(|_| {
        BehaviourBuilder::default()
            .with_core_peer_membership(PeerId::random())
            .build()
    });
}
