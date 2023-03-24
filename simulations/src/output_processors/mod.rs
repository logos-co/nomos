use serde::Serialize;

pub type SerializedNodeState = serde_json::Value;

#[derive(Serialize)]
pub struct OutData {}

pub trait NodeStateRecord {
    fn get_serialized_state_record(&self) -> SerializedNodeState {
        SerializedNodeState::Null
    }
}
