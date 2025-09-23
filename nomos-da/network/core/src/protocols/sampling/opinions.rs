use libp2p::PeerId;
use nomos_core::block::SessionNumber;

#[derive(Debug, Clone)]
pub enum OpinionEvent {
    Positive { peer_id: PeerId, session: Session },
    Negative { peer_id: PeerId, session: Session },
    Blacklist { peer_id: PeerId, session: Session },
}

#[derive(Debug, Clone)]
pub enum Session {
    Current,
    Id(SessionNumber),
}
