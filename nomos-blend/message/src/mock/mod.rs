pub mod error;

use error::Error;

use crate::{BlendMessage, MessageUnwrapError};

const NODE_ID_SIZE: usize = 32;
// TODO: Move MAX_PAYLOAD_SIZE and MAX_LAYERS to the upper layer (service
// layer).
const MAX_PAYLOAD_SIZE: usize = 2048;
const PAYLOAD_PADDING_SEPARATOR: u8 = 0x01;
const PAYLOAD_PADDING_SEPARATOR_SIZE: usize = 1;
const MAX_LAYERS: usize = 5;
pub const MESSAGE_SIZE: usize =
    NODE_ID_SIZE * MAX_LAYERS + MAX_PAYLOAD_SIZE + PAYLOAD_PADDING_SEPARATOR_SIZE;
const DUMMY_NODE_ID: [u8; NODE_ID_SIZE] = [u8::MAX; NODE_ID_SIZE];

/// A mock implementation of the Sphinx encoding.
#[derive(Clone, Debug)]
pub struct MockBlendMessage;

impl BlendMessage for MockBlendMessage {
    type PublicKey = [u8; NODE_ID_SIZE];
    type PrivateKey = [u8; NODE_ID_SIZE];
    type Error = Error;
    const DROP_MESSAGE: &'static [u8] = &[0; MESSAGE_SIZE];

    /// The length of the encoded message is fixed to [`MESSAGE_SIZE`] bytes.
    /// The [`MAX_LAYERS`] number of [`NodeId`]s are concatenated in front of
    /// the payload. The payload is zero-padded to the end.
    fn build(payload: &[u8], public_keys: &[Self::PublicKey]) -> Result<Vec<u8>, Self::Error> {
        // In this mock, we don't encrypt anything. So, we use public key as just a node
        // ID.
        let node_ids = public_keys;
        if node_ids.is_empty() || node_ids.len() > MAX_LAYERS {
            return Err(Error::InvalidNumberOfLayers);
        }
        if payload.len() > MAX_PAYLOAD_SIZE {
            return Err(Error::PayloadTooLarge);
        }

        let mut message: Vec<u8> = Vec::with_capacity(MESSAGE_SIZE);

        for node_id in node_ids {
            if node_id == &DUMMY_NODE_ID {
                return Err(Error::InvalidPublicKey);
            }
            message.extend(node_id);
        }
        // If there is any remaining layers, fill them with [`DUMMY_NODE_ID`].
        (0..MAX_LAYERS - node_ids.len()).for_each(|_| message.extend(&DUMMY_NODE_ID));

        // Append payload with padding
        message.extend(payload);
        message.push(PAYLOAD_PADDING_SEPARATOR);
        message.extend(std::iter::repeat_n(0, MAX_PAYLOAD_SIZE - payload.len()));
        Ok(message)
    }

    fn unwrap(
        message: &[u8],
        private_key: &Self::PrivateKey,
    ) -> Result<(Vec<u8>, bool), MessageUnwrapError<Self::Error>> {
        if message.len() != MESSAGE_SIZE {
            return Err(Error::InvalidBlendMessage.into());
        }

        // In this mock, we don't decrypt anything. So, we use private key as just a
        // node ID.
        let node_id = private_key;
        if &message[0..NODE_ID_SIZE] != node_id {
            return Err(MessageUnwrapError::NotAllowed);
        }

        // If this is the last layer
        if message[NODE_ID_SIZE..NODE_ID_SIZE * 2] == DUMMY_NODE_ID {
            return Ok((Self::payload(message)?, true));
        }

        let mut new_message: Vec<u8> = Vec::with_capacity(MESSAGE_SIZE);
        new_message.extend(&message[NODE_ID_SIZE..NODE_ID_SIZE * MAX_LAYERS]);
        new_message.extend(&DUMMY_NODE_ID);
        new_message.extend(&message[NODE_ID_SIZE * MAX_LAYERS..]); // padded payload
        Ok((new_message, false))
    }
}

impl MockBlendMessage {
    pub fn payload(message: &[u8]) -> Result<Vec<u8>, Error> {
        let padded_payload = &message[NODE_ID_SIZE * MAX_LAYERS..];
        // remove the payload padding
        padded_payload
            .iter()
            .rposition(|&x| x == PAYLOAD_PADDING_SEPARATOR)
            .map_or(Err(Error::InvalidBlendMessage), |pos| {
                Ok(padded_payload[0..pos].to_vec())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message() {
        let node_ids = (0..3).map(|i| [i; NODE_ID_SIZE]).collect::<Vec<_>>();
        let payload = [7; 10];
        let message = MockBlendMessage::build(&payload, &node_ids).unwrap();
        assert_eq!(message.len(), MESSAGE_SIZE);
        assert_eq!(MockBlendMessage::payload(&message).unwrap(), payload);

        let (message, is_fully_unwrapped) =
            MockBlendMessage::unwrap(&message, &node_ids[0]).unwrap();
        assert!(!is_fully_unwrapped);
        assert_eq!(message.len(), MESSAGE_SIZE);
        assert_eq!(MockBlendMessage::payload(&message).unwrap(), payload);

        let (message, is_fully_unwrapped) =
            MockBlendMessage::unwrap(&message, &node_ids[1]).unwrap();
        assert!(!is_fully_unwrapped);
        assert_eq!(message.len(), MESSAGE_SIZE);
        assert_eq!(MockBlendMessage::payload(&message).unwrap(), payload);

        let (unwrapped_payload, is_fully_unwrapped) =
            MockBlendMessage::unwrap(&message, &node_ids[2]).unwrap();
        assert!(is_fully_unwrapped);
        assert_eq!(unwrapped_payload, payload);
    }

    #[test]
    fn invalid_node_id() {
        let mut node_ids = (0..3).map(|i| [i; NODE_ID_SIZE]).collect::<Vec<_>>();
        node_ids.push(DUMMY_NODE_ID);
        let payload = [7; 10];
        assert_eq!(
            MockBlendMessage::build(&payload, &node_ids),
            Err(Error::InvalidPublicKey)
        );
    }
}
