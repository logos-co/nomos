use nomos_core::mantle::ops::channel::ChannelId;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct DispersalRequest<Metadata> {
    pub channel_id: ChannelId,
    pub data: Vec<u8>,
    pub metadata: Metadata,
}
