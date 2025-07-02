pub mod core_edge;
pub use core_edge::CoreToEdgeBlendConnectionHandler;

pub mod edge_core;
// TODO: Remove test flag once the component is integrated.
#[cfg(test)]
pub use edge_core::EdgeToCoreBlendConnectionHandler;
