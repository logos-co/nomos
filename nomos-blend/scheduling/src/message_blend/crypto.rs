use derivative::Derivative;
use nomos_blend_message::{
    crypto::{Ed25519PrivateKey, ProofOfQuota, ProofOfSelection, X25519PrivateKey},
    encap::{DecapsulationOutput, EncapsulatedMessage},
    input::{EncapsulationInput, EncapsulationInputs},
    Error, PayloadType,
};
use nomos_core::wire;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::{membership::Membership, serde::ed25519_privkey_hex, BlendOutgoingMessage};

const ENCAPSULATION_COUNT: usize = 3;

/// [`CryptographicProcessor`] is responsible for wrapping and unwrapping
/// messages for the message indistinguishability.
pub struct CryptographicProcessor<NodeId, Rng> {
    settings: CryptographicProcessorSettings,
    /// The non-ephemeral encryption key for decapsulating messages.
    encryption_private_key: X25519PrivateKey,
    membership: Membership<NodeId>,
    rng: Rng,
}

#[derive(Clone, Derivative, Serialize, Deserialize)]
#[derivative(Debug)]
pub struct CryptographicProcessorSettings {
    /// The non-ephemeral signing key corresponding to the public key
    /// registered in the membership (SDP).
    #[serde(with = "ed25519_privkey_hex")]
    #[derivative(Debug = "ignore")]
    pub signing_private_key: Ed25519PrivateKey,
    /// `ß_c`: expected number of blending operations for each locally generated
    /// message.
    pub num_blend_layers: u64,
}

impl<NodeId, Rng> CryptographicProcessor<NodeId, Rng> {
    pub fn new(
        settings: CryptographicProcessorSettings,
        membership: Membership<NodeId>,
        rng: Rng,
    ) -> Self {
        // Derive the non-ephemeral encryption key
        // from the non-ephemeral signing key.
        let encryption_private_key = settings.signing_private_key.derive_x25519();
        Self {
            settings,
            encryption_private_key,
            membership,
            rng,
        }
    }

    pub fn decapsulate_message(&self, message: &[u8]) -> Result<BlendOutgoingMessage, Error> {
        deserialize_encapsulated_message(message)?
            .decapsulate(&self.encryption_private_key)
            .map(BlendOutgoingMessage::from)
    }
}

impl<NodeId, Rng> CryptographicProcessor<NodeId, Rng>
where
    Rng: RngCore,
{
    pub fn encapsulate_cover_message(&mut self, payload: &[u8]) -> Result<Vec<u8>, Error> {
        self.encapsulate_message(PayloadType::Cover, payload)
    }

    pub fn encapsulate_data_message(&mut self, payload: &[u8]) -> Result<Vec<u8>, Error> {
        self.encapsulate_message(PayloadType::Data, payload)
    }

    fn encapsulate_message(
        &mut self,
        payload_type: PayloadType,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        // Retrieve the non-ephemeral signing keys of the blend nodes
        let blend_node_signing_keys = self
            .membership
            .choose_remote_nodes(&mut self.rng, self.settings.num_blend_layers as usize)
            .map(|node| node.public_key.clone())
            .collect::<Vec<_>>();

        let inputs = EncapsulationInputs::<ENCAPSULATION_COUNT>::new(
            blend_node_signing_keys
                .iter()
                .map(|blend_node_signing_key| {
                    // Generate an ephemeral signing key for each
                    // encapsulation.
                    let ephemeral_signing_key = Ed25519PrivateKey::generate();
                    EncapsulationInput::new(
                        ephemeral_signing_key,
                        blend_node_signing_key,
                        ProofOfQuota::dummy(),
                        ProofOfSelection::dummy(),
                    )
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )?;

        let message =
            EncapsulatedMessage::<ENCAPSULATION_COUNT>::new(&inputs, payload_type, payload)?;
        Ok(serialize_encapsulated_message(&message))
    }
}

impl From<DecapsulationOutput<ENCAPSULATION_COUNT>> for BlendOutgoingMessage {
    fn from(output: DecapsulationOutput<ENCAPSULATION_COUNT>) -> Self {
        match output {
            DecapsulationOutput::Incompleted(message) => {
                Self::EncapsulatedMessage(serialize_encapsulated_message(&message).into())
            }
            DecapsulationOutput::Completed((payload_type, payload_body)) => match payload_type {
                PayloadType::Cover => Self::CoverMessage(payload_body.into()),
                PayloadType::Data => Self::DataMessage(payload_body.into()),
            },
        }
    }
}

fn serialize_encapsulated_message(message: &EncapsulatedMessage<ENCAPSULATION_COUNT>) -> Vec<u8> {
    wire::serialize(&message).expect("EncapsulatedMessage should be serializable")
}

fn deserialize_encapsulated_message(
    message: &[u8],
) -> Result<EncapsulatedMessage<ENCAPSULATION_COUNT>, Error> {
    wire::deserialize(message).map_err(|_| Error::DeserializationFailed)
}
