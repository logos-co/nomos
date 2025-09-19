use core::{
    iter::repeat_with,
    ops::{Deref, DerefMut},
    time::Duration,
};

use libp2p::{
    core::transport::MemoryTransport, identity, plaintext, swarm, tcp, yamux, PeerId,
    StreamProtocol, Swarm, Transport as _,
};
use nomos_blend_message::{
    crypto::{
        keys::Ed25519PrivateKey,
        proofs::{quota::ProofOfQuota, selection::ProofOfSelection},
    },
    input::EncapsulationInput,
    PayloadType,
};
use nomos_blend_scheduling::{message_blend::crypto::EncapsulationInputs, EncapsulatedMessage};
use nomos_libp2p::{upgrade::Version, NetworkBehaviour};

pub const PROTOCOL_NAME: StreamProtocol = StreamProtocol::new("/blend/swarm/test");

#[derive(Debug)]
pub struct TestEncapsulatedMessage(EncapsulatedMessage);

impl TestEncapsulatedMessage {
    pub fn new(payload: &[u8]) -> Self {
        Self(
            EncapsulatedMessage::new(&generate_valid_inputs(), PayloadType::Data, payload).unwrap(),
        )
    }

    pub fn into_inner(self) -> EncapsulatedMessage {
        self.0
    }
}

impl Deref for TestEncapsulatedMessage {
    type Target = EncapsulatedMessage;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TestEncapsulatedMessage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

fn generate_valid_inputs() -> EncapsulationInputs {
    EncapsulationInputs::new(
        repeat_with(Ed25519PrivateKey::generate)
            .take(3)
            .map(|recipient_signing_key| {
                let recipient_signing_pubkey = recipient_signing_key.public_key();
                EncapsulationInput::new(
                    Ed25519PrivateKey::generate(),
                    &recipient_signing_pubkey,
                    ProofOfQuota::from_bytes_unchecked([0; _]),
                    ProofOfSelection::from_bytes_unchecked([0; _]),
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
    .unwrap()
}

/// Instantiate a new memory-based Swarm that uses the configured timeout for
/// idle connections and instantiates the behaviour as returned by the provided
/// constructor.
pub fn memory_test_swarm<BehaviourConstructor, Behaviour>(
    idle_connection_timeout: Duration,
    behaviour_constructor: BehaviourConstructor,
) -> Swarm<Behaviour>
where
    BehaviourConstructor: FnOnce(identity::Keypair) -> Behaviour,
    Behaviour: NetworkBehaviour,
{
    let identity = identity::Keypair::generate_ed25519();
    let peer_id = PeerId::from(identity.public());

    let transport = MemoryTransport::default()
        .or_transport(tcp::tokio::Transport::default())
        .upgrade(Version::V1)
        .authenticate(plaintext::Config::new(&identity))
        .multiplex(yamux::Config::default())
        .timeout(Duration::from_secs(1))
        .boxed();

    Swarm::new(
        transport,
        behaviour_constructor(identity),
        peer_id,
        swarm::Config::with_tokio_executor().with_idle_connection_timeout(idle_connection_timeout),
    )
}
