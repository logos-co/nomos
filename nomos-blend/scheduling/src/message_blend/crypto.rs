use derivative::Derivative;
use nomos_blend_message::{
    crypto::{Ed25519PrivateKey, ProofOfQuota, ProofOfSelection, X25519PrivateKey},
    encap::{
        DecapsulationOutput as InternalDecapsulationOutput,
        EncapsulatedMessage as InternalEncapsulatedMessage,
    },
    input::{EncapsulationInput, EncapsulationInputs as InternalEncapsulationInputs},
    Error, PayloadType,
};
use nomos_core::wire;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::{membership::Membership, serde::ed25519_privkey_hex};

const ENCAPSULATION_COUNT: usize = 3;
pub type EncapsulatedMessage = InternalEncapsulatedMessage<ENCAPSULATION_COUNT>;
pub type EncapsulationInputs = InternalEncapsulationInputs<ENCAPSULATION_COUNT>;
pub type UnwrappedMessage = InternalDecapsulationOutput<ENCAPSULATION_COUNT>;

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

    pub fn decapsulate_serialized_message(
        &self,
        message: &[u8],
    ) -> Result<UnwrappedMessage, Error> {
        self.decapsulate_message(deserialize_encapsulated_message(message)?)
    }

    pub fn decapsulate_message(
        &self,
        message: EncapsulatedMessage,
    ) -> Result<UnwrappedMessage, Error> {
        message.decapsulate(&self.encryption_private_key)
    }
}

impl<NodeId, Rng> CryptographicProcessor<NodeId, Rng>
where
    Rng: RngCore,
{
    pub fn encapsulate_cover_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        self.encapsulate_payload(PayloadType::Cover, payload)
    }

    pub fn encapsulate_and_serialize_cover_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        Ok(serialize_encapsulated_message(
            &self.encapsulate_cover_payload(payload)?,
        ))
    }

    pub fn encapsulate_data_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        self.encapsulate_payload(PayloadType::Data, payload)
    }

    pub fn encapsulate_and_serialize_data_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        Ok(serialize_encapsulated_message(
            &self.encapsulate_data_payload(payload)?,
        ))
    }

    fn encapsulate_payload(
        &mut self,
        payload_type: PayloadType,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        // Retrieve the non-ephemeral signing keys of the blend nodes
        let blend_node_signing_keys = self
            .membership
            .choose_remote_nodes(&mut self.rng, self.settings.num_blend_layers as usize)
            .map(|node| node.public_key)
            .collect::<Vec<_>>();

        let inputs = EncapsulationInputs::new(
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

        EncapsulatedMessage::new(&inputs, payload_type, payload)
    }
}

#[must_use]
pub fn serialize_encapsulated_message(message: &EncapsulatedMessage) -> Vec<u8> {
    wire::serialize(&message).expect("EncapsulatedMessage should be serializable")
}

pub fn deserialize_encapsulated_message(message: &[u8]) -> Result<EncapsulatedMessage, Error> {
    wire::deserialize(message).map_err(|_| Error::DeserializationFailed)
}
