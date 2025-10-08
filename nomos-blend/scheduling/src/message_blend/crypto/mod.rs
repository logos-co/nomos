use derivative::Derivative;
use nomos_blend_message::{
    Error,
    crypto::keys::Ed25519PrivateKey,
    encap::{
        decapsulated::DecapsulationOutput as InternalDecapsulationOutput,
        encapsulated::EncapsulatedMessage as InternalEncapsulatedMessage,
        validated::{
            IncomingEncapsulatedMessageWithValidatedPublicHeader as InternalIncomingEncapsulatedMessageWithValidatedPublicHeader,
            OutgoingEncapsulatedMessageWithValidatedPublicHeader as InternalOutgoingEncapsulatedMessageWithValidatedPublicHeader,
        },
    },
    input::EncapsulationInputs as InternalEncapsulationInputs,
};
use nomos_core::codec::{DeserializeOp as _, SerializeOp as _};

pub mod send;
pub use self::send::SessionCryptographicProcessor as SenderOnlySessionCryptographicProcessor;
pub mod send_and_receive;
pub use self::send_and_receive::SessionCryptographicProcessor as SendAndReceiveSessionCryptographicProcessor;

const ENCAPSULATION_COUNT: usize = 3;
pub type EncapsulatedMessage = InternalEncapsulatedMessage<ENCAPSULATION_COUNT>;
pub type EncapsulationInputs = InternalEncapsulationInputs<ENCAPSULATION_COUNT>;
pub type DecapsulationOutput = InternalDecapsulationOutput<ENCAPSULATION_COUNT>;
pub type IncomingEncapsulatedMessageWithValidatedPublicHeader =
    InternalIncomingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>;
pub type OutgoingEncapsulatedMessageWithValidatedPublicHeader =
    InternalOutgoingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>;

#[derive(Clone, Derivative, serde::Serialize, serde::Deserialize)]
#[derivative(Debug)]
pub struct SessionCryptographicProcessorSettings {
    /// The non-ephemeral signing key (NSK) corresponding to the public key
    /// registered in the membership (SDP).
    #[serde(with = "crate::serde::ed25519_privkey_hex")]
    #[derivative(Debug = "ignore")]
    pub non_ephemeral_signing_key: Ed25519PrivateKey,
    /// `ß_c`: expected number of blending operations for each locally generated
    /// message.
    pub num_blend_layers: u64,
}

#[must_use]
pub fn serialize_encapsulated_message(message: &EncapsulatedMessage) -> Vec<u8> {
    message
        .to_bytes()
        .expect("EncapsulatedMessage should be serializable")
        .to_vec()
}

pub fn deserialize_encapsulated_message(message: &[u8]) -> Result<EncapsulatedMessage, Error> {
    EncapsulatedMessage::from_bytes(message).map_err(|_| Error::DeserializationFailed)
}
