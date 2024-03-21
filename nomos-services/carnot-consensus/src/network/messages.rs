// std
// crates
use serde::{Deserialize, Serialize};
// internal
use crate::NodeId;
use crate::{NewView, Qc, Timeout, TimeoutQc, Vote};
use carnot_engine::View;
use nomos_core::header::HeaderId;
use nomos_core::wire;

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq, Hash)]
pub struct ProposalMsg {
    pub data: Box<[u8]>,
    pub proposal: HeaderId,
    pub view: View,
}

impl ProposalMsg {
    pub fn as_bytes(&self) -> Box<[u8]> {
        wire::serialize(self).unwrap().into_boxed_slice()
    }

    pub fn from_bytes(data: &[u8]) -> Self {
        wire::deserialize(data).unwrap()
    }
}

#[derive(Debug, Eq, PartialEq, Hash, Serialize, Deserialize, Clone)]
pub struct VoteMsg {
    pub voter: NodeId,
    pub vote: Vote,
    pub qc: Option<Qc>,
}

impl VoteMsg {
    pub fn as_bytes(&self) -> Box<[u8]> {
        wire::serialize(self).unwrap().into_boxed_slice()
    }
    pub fn from_bytes(data: &[u8]) -> Self {
        wire::deserialize(data).unwrap()
    }
}

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq, Clone)]
pub struct NewViewMsg {
    pub voter: NodeId,
    pub vote: NewView,
}

impl NewViewMsg {
    pub fn as_bytes(&self) -> Box<[u8]> {
        wire::serialize(self).unwrap().into_boxed_slice()
    }
    pub fn from_bytes(data: &[u8]) -> Self {
        wire::deserialize(data).unwrap()
    }
}

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq, Clone)]
pub struct TimeoutMsg {
    pub voter: NodeId,
    pub vote: Timeout,
}

impl TimeoutMsg {
    pub fn as_bytes(&self) -> Box<[u8]> {
        wire::serialize(self).unwrap().into_boxed_slice()
    }
    pub fn from_bytes(data: &[u8]) -> Self {
        wire::deserialize(data).unwrap()
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Clone)]
pub struct TimeoutQcMsg {
    pub source: NodeId,
    pub qc: TimeoutQc,
}

impl TimeoutQcMsg {
    pub fn as_bytes(&self) -> Box<[u8]> {
        wire::serialize(self).unwrap().into_boxed_slice()
    }
    pub fn from_bytes(data: &[u8]) -> Self {
        wire::deserialize(data).unwrap()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum NetworkMessage {
    Timeout(TimeoutMsg),
    TimeoutQc(TimeoutQcMsg),
    Vote(VoteMsg),
    NewView(NewViewMsg),
    Proposal(ProposalMsg),
}

impl NetworkMessage {
    pub fn as_bytes(&self) -> Box<[u8]> {
        wire::serialize(self).unwrap().into_boxed_slice()
    }
    pub fn from_bytes(data: &[u8]) -> Self {
        wire::deserialize(data).unwrap()
    }
}
