use nomos_utils::blake_rng::BlakeRng;

use crate::{BlendService, core, edge, membership};

/// Exposes associated types for external modules that depend on
/// [`BlendService`], without requiring them to specify its generic parameters.
pub trait ServiceComponents {
    /// Settings for broadcasting messages that have passed through the blend
    /// network.
    type BroadcastSettings;
}

impl<
    CoreBackend,
    EdgeBackend,
    BroadcastSettings,
    MembershipAdapter,
    ChainService,
    NetworkAdapter,
    TimeService,
    PolInfoProvider,
    CoreProofsGenerator,
    EdgeProofsGenerator,
    ProofsVerifier,
    RuntimeServiceId,
> ServiceComponents
    for BlendService<
        CoreBackend,
        EdgeBackend,
        BroadcastSettings,
        MembershipAdapter,
        ChainService,
        NetworkAdapter,
        TimeService,
        PolInfoProvider,
        CoreProofsGenerator,
        EdgeProofsGenerator,
        ProofsVerifier,
        RuntimeServiceId,
    >
where
    CoreBackend: core::backends::BlendBackend<
            <MembershipAdapter as membership::Adapter>::NodeId,
            BlakeRng,
            ProofsVerifier,
            RuntimeServiceId,
        >,
    EdgeBackend: edge::backends::BlendBackend<
            <MembershipAdapter as membership::Adapter>::NodeId,
            RuntimeServiceId,
        >,
    MembershipAdapter: membership::Adapter<NodeId: Clone>,
{
    type BroadcastSettings = BroadcastSettings;
}
