use nomos_core::mantle::ops::channel::{ChannelId, Ed25519PublicKey, MsgId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelState {
    pub channel_id: ChannelId,
    pub tip: MsgId,
    pub accredited_keys: Vec<Ed25519PublicKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub id: MsgId,
    pub channel_id: ChannelId,
    pub parent: MsgId,
    pub signer: Ed25519PublicKey,
    pub block_height: u64,
    pub operation_type: String, // "inscribe" or "blob"
    pub content: ChannelMessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChannelMessageContent {
    Inscription { data: Vec<u8> },
    Blob { blob_id: [u8; 32], blob_size: u64 },
}
