use libp2p::PeerId;
use nomos_core::block::SessionNumber;

#[derive(Debug, Clone)]
pub enum OpinionEvent {
    Positive {
        peer_id: PeerId,
        session_id: Option<SessionNumber>, // None = current session
    },
    Negative {
        peer_id: PeerId,
        session_id: Option<SessionNumber>,
    },
    Blacklist {
        peer_id: PeerId,
        session_id: Option<SessionNumber>,
    },
}
