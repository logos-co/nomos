use std::{env, net::Ipv4Addr, ops::Mul as _, sync::LazyLock, time::Duration};

use nomos_core::sdp::ProviderId;
use nomos_libp2p::{Multiaddr, PeerId, multiaddr};

pub mod common;
pub mod nodes;
pub mod topology;

static IS_SLOW_TEST_ENV: LazyLock<bool> =
    LazyLock::new(|| env::var("SLOW_TEST_ENV").is_ok_and(|s| s == "true"));

pub static IS_DEBUG_TRACING: LazyLock<bool> = LazyLock::new(|| {
    env::var("NOMOS_TESTS_TRACING").is_ok_and(|val| val.eq_ignore_ascii_case("true"))
});

/// In slow test environments like Codecov, use 2x timeout.
#[must_use]
pub fn adjust_timeout(d: Duration) -> Duration {
    if *IS_SLOW_TEST_ENV { d.mul(2) } else { d }
}

#[must_use]
pub fn node_address_from_port(port: u16) -> Multiaddr {
    multiaddr(Ipv4Addr::LOCALHOST, port)
}

#[must_use]
pub fn secret_key_to_peer_id(node_key: nomos_libp2p::ed25519::SecretKey) -> PeerId {
    PeerId::from_public_key(
        &nomos_libp2p::ed25519::Keypair::from(node_key)
            .public()
            .into(),
    )
}

#[must_use]
pub fn secret_key_to_provider_id(node_key: nomos_libp2p::ed25519::SecretKey) -> ProviderId {
    ProviderId::try_from(
        nomos_libp2p::ed25519::Keypair::from(node_key)
            .public()
            .to_bytes(),
    )
    .unwrap()
}
