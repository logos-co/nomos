pub mod with_core;
pub mod with_edge;

use libp2p::PeerId;
use nomos_blend_scheduling::{
    membership::Membership, message_blend::crypto::CryptographicProcessor,
};

use self::{
    with_core::behaviour::Behaviour as CoreToCoreBehaviour,
    with_edge::behaviour::Behaviour as CoreToEdgeBehaviour,
};
use crate::core::with_edge::behaviour::Config as CoreToEdgeConfig;

/// A composed behaviour that wraps the two sub-behaviours for dealing with core
/// and edge nodes.
#[derive(nomos_libp2p::NetworkBehaviour)]
pub struct NetworkBehaviour<Rng, ObservationWindowClockProvider> {
    with_core: CoreToCoreBehaviour<Rng, ObservationWindowClockProvider>,
    with_edge: CoreToEdgeBehaviour,
}

impl<Rng, ObservationWindowClockProvider> NetworkBehaviour<Rng, ObservationWindowClockProvider> {
    pub const fn with_core(&self) -> &CoreToCoreBehaviour<Rng, ObservationWindowClockProvider> {
        &self.with_core
    }

    pub const fn with_core_mut(
        &mut self,
    ) -> &mut CoreToCoreBehaviour<Rng, ObservationWindowClockProvider> {
        &mut self.with_core
    }

    pub const fn with_edge(&self) -> &CoreToEdgeBehaviour {
        &self.with_edge
    }

    pub const fn with_edge_mut(&mut self) -> &mut CoreToEdgeBehaviour {
        &mut self.with_edge
    }
}

pub struct Config {
    pub with_edge: CoreToEdgeConfig,
}

impl<Rng, ObservationWindowClockProvider> NetworkBehaviour<Rng, ObservationWindowClockProvider> {
    pub fn new(
        config: &Config,
        observation_window_clock_provider: ObservationWindowClockProvider,
        current_membership: Option<Membership<PeerId>>,
        cryptographic_processor: CryptographicProcessor<PeerId, Rng>,
    ) -> Self {
        Self {
            with_core: CoreToCoreBehaviour::new(
                observation_window_clock_provider,
                current_membership.clone(),
                cryptographic_processor,
            ),
            with_edge: CoreToEdgeBehaviour::new(&config.with_edge, current_membership),
        }
    }
}
