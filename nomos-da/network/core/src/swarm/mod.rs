pub(crate) mod common;
pub mod executor;
pub mod validator;

pub use common::{
    balancer::BalancerStats,
    monitor::{dto::MonitorStats, DAConnectionMonitorSettings},
    policy::DAConnectionPolicySettings,
    ReplicationConfig,
};
use futures::channel::oneshot;

use crate::protocols::dispersal::validator::behaviour::DispersalEvent;

pub(crate) type ConnectionMonitor<Membership> =
    common::monitor::DAConnectionMonitor<common::policy::DAConnectionPolicy<Membership>>;

pub(crate) type ConnectionBalancer<Membership> = common::balancer::DAConnectionBalancer<
    Membership,
    common::policy::DAConnectionPolicy<Membership>,
>;

/// Dispersed data failed to be validated.
pub struct DispersalValidationError;

pub type DispersalValidationResult = Result<(), DispersalValidationError>;

pub type ValidationResultSender = Option<oneshot::Sender<DispersalValidationResult>>;

pub struct DispersalValidatorEvent {
    pub event: DispersalEvent,
    pub sender: ValidationResultSender,
}
