use std::time::{Duration, SystemTime};

use lb_libp2p::PeerId;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot};

/// The service or subsystem that reported a peer offense.
///
/// `Other` deliberately owns its label so that the reporting scope is not
/// lost when a violation crosses the service boundary.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum Subsystem {
    ChainSync,
    Libp2p,
    DataAvailability,
    Blend,
    Other(String),
}

/// Provenance of a ban independently from where it is enforced.
///
/// This is descriptive provenance, not an ownership dimension: each
/// `(PeerId, BanScope)` has one effective record, and reports from different
/// sources combine through the normal report/replace semantics rather than
/// creating independently removable leases. The service does not arbitrate
/// which source is authorized to govern a scope.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum BanSource {
    Service(Subsystem),
    Configuration,
}

/// Scope in which a ban is enforced.
///
/// `Global` is an explicit network-wide policy scope. A service-scoped ban
/// applies only when that service opts into enforcement. Each `(PeerId,
/// BanScope)` has one effective record; scopes for the same peer coexist
/// independently.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum BanScope {
    Global,
    Service(Subsystem),
}

/// Descriptive metadata for the behavior reported by a functional subsystem.
///
/// The reporting subsystem owns the policy decision, including whether the
/// behavior warrants a ban and which duration to request. This taxonomy is
/// illustrative rather than an exhaustive or centrally configured policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum OffenseKind {
    SpamMsg,
    InvalidSig,
    ProtocolViolation,
    TooManyDials,
    DoSConnectionFlood,
    InvalidBlob,
    GossipsubScoreDrop,
    Other,
    BlackListed,
}

/// A complete report submitted to the banning service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Violation {
    pub peer_id: PeerId,
    pub source: BanSource,
    pub scope: BanScope,
    pub offense: OffenseKind,
    pub duration: Duration,
    pub context: Option<String>,
}

impl Violation {
    /// Construct a report with explicit provenance and enforcement scope.
    #[must_use]
    pub const fn new(
        peer_id: PeerId,
        source: BanSource,
        scope: BanScope,
        offense: OffenseKind,
        duration: Duration,
        context: Option<String>,
    ) -> Self {
        Self {
            peer_id,
            source,
            scope,
            offense,
            duration,
            context,
        }
    }
}

/// The in-memory record retained for an active ban.
///
/// A record is the effective policy lease for its `(peer_id, scope)` pair.
/// `source` preserves the provenance of the report that established or
/// extended that lease; it is not a separately removable owner.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BanRecord {
    pub peer_id: PeerId,
    pub source: BanSource,
    pub scope: BanScope,
    pub offense: OffenseKind,
    pub context: Option<String>,
    pub reported_at: SystemTime,
    /// `None` is reserved for configured blacklist entries.
    pub expires_at: Option<SystemTime>,
}

impl BanRecord {
    #[must_use]
    pub fn applies_to(&self, scope: &BanScope) -> bool {
        self.scope == BanScope::Global || &self.scope == scope
    }
}

/// Events emitted for changes to active ban state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BanEvent {
    Banned(BanRecord),
    Replaced(BanRecord),
    Unbanned { record: BanRecord, expired: bool },
}

/// Requests handled by [`crate::BanningService`].
#[derive(Debug)]
pub enum BanningRequest {
    BanPeer {
        violation: Violation,
        reply: oneshot::Sender<Option<BanRecord>>,
    },
    ReplaceBan {
        violation: Violation,
        reply: oneshot::Sender<Option<BanRecord>>,
    },
    ListActive {
        reply: oneshot::Sender<Vec<BanRecord>>,
    },
    UnbanPeer {
        peer_id: PeerId,
        scope: Option<BanScope>,
        reply: oneshot::Sender<bool>,
    },
    Subscribe {
        reply: oneshot::Sender<broadcast::Receiver<BanEvent>>,
    },
}
