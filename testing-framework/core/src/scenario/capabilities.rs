use async_trait::async_trait;

use super::DynError;

/// Marker type used by scenario builders to request node control support.
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeControlCapability;

/// Trait implemented by scenario capability markers to signal whether node
/// control is required.
pub trait RequiresNodeControl {
    const REQUIRED: bool;
}

impl RequiresNodeControl for () {
    const REQUIRED: bool = false;
}

impl RequiresNodeControl for NodeControlCapability {
    const REQUIRED: bool = true;
}

/// Interface exposed by runners that can restart nodes at runtime.
#[async_trait]
pub trait NodeControlHandle: Send + Sync {
    async fn restart_validator(&self, index: usize) -> Result<(), DynError>;
    async fn restart_executor(&self, index: usize) -> Result<(), DynError>;
}
