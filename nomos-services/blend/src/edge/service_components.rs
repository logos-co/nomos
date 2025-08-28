use crate::edge::{backends::BlendBackend, BlendService};

/// Exposes associated types for external modules that depend on
/// [`BlendService`], without requiring them to specify its generic parameters.
pub trait ServiceComponents {
    /// Settings for broadcasting messages that have passed through the blend
    /// network.
    type BroadcastSettings;
}

impl<Backend, NodeId, BroadcastSettings, RuntimeServiceId> ServiceComponents
    for BlendService<Backend, NodeId, BroadcastSettings, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    type BroadcastSettings = BroadcastSettings;
}
