use error::Error;
use packet::{Packet, UnpackedPacket};

use crate::{BlendMessage, MessageUnwrapError};

pub mod error;
mod layered_cipher;
pub mod packet;
mod routing;

#[derive(Clone, Debug)]
pub struct SphinxMessage;

const ASYM_KEY_SIZE: usize = 32;
// TODO: Move these constants to the upper layer (service layer).
const MAX_PAYLOAD_SIZE: usize = 2048;
const MAX_LAYERS: usize = 5;

impl BlendMessage for SphinxMessage {
    type PublicKey = [u8; ASYM_KEY_SIZE];
    type PrivateKey = [u8; ASYM_KEY_SIZE];
    type Error = Error;
    fn build(payload: &[u8], public_keys: &[Self::PublicKey]) -> Result<Vec<u8>, Self::Error> {
        let packet = Packet::build(
            &public_keys
                .iter()
                .map(|k| x25519_dalek::PublicKey::from(*k))
                .collect::<Vec<_>>(),
            MAX_LAYERS,
            payload,
            MAX_PAYLOAD_SIZE,
        )?;
        Ok(packet.to_bytes())
    }

    fn unwrap(
        message: &[u8],
        private_key: &Self::PrivateKey,
    ) -> Result<(Vec<u8>, bool), MessageUnwrapError<Self::Error>> {
        let packet = Packet::from_bytes(message, MAX_LAYERS)?;
        let unpacked_packet =
            packet.unpack(&x25519_dalek::StaticSecret::from(*private_key), MAX_LAYERS)?;
        match unpacked_packet {
            UnpackedPacket::ToForward(packet) => Ok((packet.to_bytes(), false)),
            UnpackedPacket::FullyUnpacked(payload) => Ok((payload, true)),
        }
    }
}

fn parse_bytes<'a>(data: &'a [u8], sizes: &[usize]) -> Result<Vec<&'a [u8]>, String> {
    let mut i = 0;
    sizes
        .iter()
        .map(|&size| {
            if i + size > data.len() {
                return Err("The sum of sizes exceeds the length of the input slice".to_owned());
            }
            let slice = &data[i..i + size];
            i += size;
            Ok(slice)
        })
        .collect()
}
