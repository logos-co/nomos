use std::io::Read;

use blake2::digest::{Update, VariableOutput};
use blake2::Blake2bVar;
use bytes::BytesMut;
use chacha20::ChaCha20;
use cipher::{KeyIvInit, StreamCipher};
use rand_chacha::{
    rand_core::{RngCore, SeedableRng},
    ChaCha12Rng,
};

pub struct Message;

const VERSION: u8 = 1;
const HEADER_SIZE: usize = 1;
const KEY_SIZE: usize = 32;
const SIGNATURE_SIZE: usize = 64;
const PROOF_OF_QUOTA_SIZE: usize = 160;
const PROOF_OF_SELECTION_SIZE: usize = 32;
const BLENDING_SUBHEADER_SIZE: usize = KEY_SIZE + SIGNATURE_SIZE + PROOF_OF_QUOTA_SIZE;
const BLENDING_HEADER_SIZE: usize = BLENDING_SUBHEADER_SIZE + PROOF_OF_SELECTION_SIZE;
const MAX_ENCAPSULATIONS: usize = 3;
const PUBLIC_HEADER_SIZE: usize = BLENDING_SUBHEADER_SIZE;
const PRIVATE_HEADER_SIZE: usize = BLENDING_HEADER_SIZE * MAX_ENCAPSULATIONS;
const PAYLOAD_HEADER_SIZE: usize = 1 + 2;
const MAX_PAYLOAD_BODY_SIZE: usize = 34 * 1024;
const PAYLOAD_SIZE: usize = PAYLOAD_HEADER_SIZE + MAX_PAYLOAD_BODY_SIZE;
const MESSAGE_SIZE: usize = HEADER_SIZE + PUBLIC_HEADER_SIZE + PRIVATE_HEADER_SIZE + PAYLOAD_SIZE;

impl Message {
    pub fn initialize(shared_keys: Vec<[u8; KEY_SIZE]>) -> BytesMut {
        if shared_keys.len() > MAX_ENCAPSULATIONS {
            panic!("Number of encapsulations exceeds the maximum allowed.");
        }

        let mut msg = BytesMut::zeroed(MESSAGE_SIZE);

        // Fill the header.
        msg[0] = VERSION;

        // Randomize the private header.
        let private_header_pos = HEADER_SIZE + PUBLIC_HEADER_SIZE;
        for i in 0..MAX_ENCAPSULATIONS {
            let bytes = pseudo_random_bytes(&random_key(), BLENDING_HEADER_SIZE);
            let offset = private_header_pos + i * BLENDING_HEADER_SIZE;
            println!(
                "Randomizing a private header at offset:{offset}, size:{BLENDING_HEADER_SIZE}"
            );
            msg[offset..offset + BLENDING_HEADER_SIZE].copy_from_slice(&bytes);
        }

        // Fill the last `shared_key.len()` blending headers with resonstructed data.
        for (i, shared_key) in shared_keys.iter().rev().enumerate() {
            let r1 = pseudo_random_bytes(
                &blake2b256(&shared_key.iter().chain(&[1]).copied().collect::<Vec<_>>()),
                KEY_SIZE,
            );
            let r2 = pseudo_random_bytes(
                &blake2b256(&shared_key.iter().chain(&[2]).copied().collect::<Vec<_>>()),
                PROOF_OF_QUOTA_SIZE,
            );
            let r3 = pseudo_random_bytes(
                &blake2b256(&shared_key.iter().chain(&[3]).copied().collect::<Vec<_>>()),
                SIGNATURE_SIZE,
            );
            let r4 = pseudo_random_bytes(
                &blake2b256(&shared_key.iter().chain(&[4]).copied().collect::<Vec<_>>()),
                PROOF_OF_SELECTION_SIZE,
            );

            let blending_header_idx = i + MAX_ENCAPSULATIONS - shared_keys.len();
            println!("Filling the {blending_header_idx}-th blending header");

            let mut offset = private_header_pos + blending_header_idx * BLENDING_HEADER_SIZE;
            println!("offset:{offset}, size:{}", r1.len());
            msg[offset..offset + r1.len()].copy_from_slice(&r1);
            offset += r1.len();
            println!("offset:{offset}, size:{}", r2.len());
            msg[offset..offset + r2.len()].copy_from_slice(&r2);
            offset += r2.len();
            println!("offset:{offset}, size:{}", r3.len());
            msg[offset..offset + r3.len()].copy_from_slice(&r3);
            offset += r3.len();
            println!("offset:{offset}, size:{}", r4.len());
            msg[offset..offset + r4.len()].copy_from_slice(&r4);
        }

        // Encrypt the last `shared_key.len()` blending headers.
        for (i, shared_key) in shared_keys.iter().enumerate() {
            let key = pseudo_random_bytes(&blake2b256(shared_key), BLENDING_HEADER_SIZE);
            for j in 0..shared_keys.len() - i {
                let blending_header_idx = j + MAX_ENCAPSULATIONS - shared_keys.len();
                let offset = private_header_pos + blending_header_idx * BLENDING_HEADER_SIZE;
                println!("Encrypting the {blending_header_idx}-th blending header with {i}-th key: offset:{offset}, size:{BLENDING_HEADER_SIZE}");
                encrypt(&mut msg[offset..offset + BLENDING_HEADER_SIZE], &key);
            }
        }

        msg
    }
}

fn pseudo_random_bytes(key: &[u8; 32], size: usize) -> Vec<u8> {
    const IV: [u8; 12] = [0u8; 12];
    let mut cipher = ChaCha20::new_from_slices(key, &IV).unwrap();
    let mut buf = vec![0u8; size];
    cipher.apply_keystream(&mut buf);
    buf
}

fn random_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    let mut rng = ChaCha12Rng::from_entropy();
    rng.fill_bytes(&mut key);
    key
}

fn blake2b256(input: &[u8]) -> [u8; 32] {
    let mut hasher = Blake2bVar::new(32).unwrap();
    hasher.update(input);
    let mut output = [0u8; 32];
    hasher.finalize_variable(&mut output).unwrap();
    output
}

fn encrypt(data: &mut [u8], key: &[u8]) {
    xor_in_place(data, key);
}

fn decrypt(data: &mut [u8], key: &[u8]) {
    encrypt(data, key);
}

fn xor_in_place(a: &mut [u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    a.iter_mut().zip(b.iter()).for_each(|(x1, &x2)| *x1 ^= x2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize() {
        let msg = Message::initialize(vec![[0u8; 32], [1u8; 32]]);
        assert_eq!(msg.len(), MESSAGE_SIZE);
        assert_eq!(msg[0], VERSION);
    }

    #[test]
    fn test_pseudo_random_bytes() {
        let size = 16;
        let bytes = pseudo_random_bytes(&[0u8; 32], size);
        assert_eq!(bytes.len(), size);
        assert_ne!(bytes, vec![0u8; size]);
    }
}
