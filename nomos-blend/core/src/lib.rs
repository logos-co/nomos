pub mod cover_traffic;
pub mod membership;
pub mod message_blend;
pub mod persistent_transmission;
mod serde;

pub enum BlendOutgoingMessage {
    CoverMessage(Vec<u8>),
    DataMessage(Vec<u8>),
    EncapsulatedMessage(Vec<u8>),
}
