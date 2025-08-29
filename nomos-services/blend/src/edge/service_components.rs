use crate::edge::{backends::BlendBackend, BlendService};

/// Exposes associated types for external modules that depend on
/// [`BlendService`], without requiring them to specify its generic parameters.
pub trait ServiceComponents {
    /// Settings for broadcasting messages that have passed through the blend
    /// network.
    type BroadcastSettings;
    /// Adapter for membership service.
    type MembershipAdapter;
}

impl<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId> ServiceComponents
    for BlendService<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    type BroadcastSettings = BroadcastSettings;
    type MembershipAdapter = MembershipAdapter;
}
