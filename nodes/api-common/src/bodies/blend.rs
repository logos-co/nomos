// TODO(RFC-480): the wallet funds the declaration stake.
// use lb_core::{mantle::NoteId, sdp::Locator};
use lb_core::{mantle::Value, sdp::Locator};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema)]
pub struct JoinBlendRequestBody {
    pub locator: Locator,
    // TODO(RFC-480): the wallet funds the declaration stake.
    // /// When omitted, the node picks a note held by its blend ZK key.
    // #[serde(default)]
    // pub service_note_id: Option<NoteId>,
    /// When omitted, the minimum stake.
    #[serde(default)]
    pub amount: Option<Value>,
}
