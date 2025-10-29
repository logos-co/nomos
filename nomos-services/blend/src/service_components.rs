use crate::{BlendService, core, edge, mode::Mode};

/// Exposes associated types for external modules that depend on
/// [`BlendService`], without requiring them to specify its generic parameters.
pub trait ServiceComponents {
    /// Settings for broadcasting messages that have passed through the blend
    /// network.
    type BroadcastSettings;
}

impl<CoreMode, EdgeMode, BroadcastMode, RuntimeServiceId> ServiceComponents
    for BlendService<CoreMode, EdgeMode, BroadcastMode, RuntimeServiceId>
where
    CoreMode: Mode<RuntimeServiceId> + core::mode_components::ModeComponents<RuntimeServiceId>,
    EdgeMode: Mode<RuntimeServiceId> + edge::mode_components::ModeComponents,
{
    type BroadcastSettings = EdgeMode::BroadcastSettings;
}
