pub mod with_core;
pub mod with_edge;

#[cfg(test)]
mod tests;

use libp2p::{PeerId, StreamProtocol};
use nomos_blend_message::encap::encapsulated::PoQVerificationInputMinusSigningKey;
use nomos_blend_scheduling::membership::Membership;

use self::{
    with_core::behaviour::Behaviour as CoreToCoreBehaviour,
    with_edge::behaviour::Behaviour as CoreToEdgeBehaviour,
};
use crate::core::{
    with_core::behaviour::Config as CoreToCoreConfig,
    with_edge::behaviour::Config as CoreToEdgeConfig,
};

/// A composed behaviour that wraps the two sub-behaviours for dealing with core
/// and edge nodes.
#[derive(nomos_libp2p::NetworkBehaviour)]
pub struct NetworkBehaviour<ProofsVerifier, ObservationWindowClockProvider> {
    with_core: CoreToCoreBehaviour<ProofsVerifier, ObservationWindowClockProvider>,
    with_edge: CoreToEdgeBehaviour<ProofsVerifier>,
}

impl<ProofsVerifier, ObservationWindowClockProvider>
    NetworkBehaviour<ProofsVerifier, ObservationWindowClockProvider>
{
    pub const fn with_core(
        &self,
    ) -> &CoreToCoreBehaviour<ProofsVerifier, ObservationWindowClockProvider> {
        &self.with_core
    }

    pub const fn with_core_mut(
        &mut self,
    ) -> &mut CoreToCoreBehaviour<ProofsVerifier, ObservationWindowClockProvider> {
        &mut self.with_core
    }

    pub const fn with_edge(&self) -> &CoreToEdgeBehaviour<ProofsVerifier> {
        &self.with_edge
    }

    pub const fn with_edge_mut(&mut self) -> &mut CoreToEdgeBehaviour<ProofsVerifier> {
        &mut self.with_edge
    }
}

pub struct Config {
    pub with_core: CoreToCoreConfig,
    pub with_edge: CoreToEdgeConfig,
}

impl<ProofsVerifier, ObservationWindowClockProvider>
    NetworkBehaviour<ProofsVerifier, ObservationWindowClockProvider>
where
    ProofsVerifier: Clone,
{
    pub fn new(
        config: &Config,
        observation_window_clock_provider: ObservationWindowClockProvider,
        current_membership: Option<Membership<PeerId>>,
        local_peer_id: PeerId,
        protocol_name: StreamProtocol,
        poq_verification_inputs: PoQVerificationInputMinusSigningKey,
        poq_verifier: ProofsVerifier,
    ) -> Self {
        Self {
            with_core: CoreToCoreBehaviour::new(
                &config.with_core,
                observation_window_clock_provider,
                current_membership.clone(),
                local_peer_id,
                protocol_name.clone(),
                poq_verification_inputs,
                poq_verifier.clone(),
            ),
            with_edge: CoreToEdgeBehaviour::new(
                &config.with_edge,
                current_membership,
                protocol_name,
                poq_verification_inputs,
                poq_verifier,
            ),
        }
    }
}
