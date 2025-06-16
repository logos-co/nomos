use blake2::digest::{Update, VariableOutput};
use blake2::Blake2bVar;
use bytes::BytesMut;
use chacha20::ChaCha20;
use cipher::{KeyIvInit, StreamCipher};

use crate::crypto::{
    pseudo_random_bytes, Ed25519PrivateKey, Ed25519PublicKey, ProofOfQuota, ProofOfSelection,
    SharedKey, X25519PrivateKey, KEY_SIZE, PROOF_OF_QUOTA_SIZE, PROOF_OF_SELECTION_SIZE,
    SIGNATURE_SIZE,
};

const VERSION: u8 = 1;
const HEADER_SIZE: usize = 1;

const BLENDING_SUBHEADER_SIZE: usize = KEY_SIZE + SIGNATURE_SIZE + PROOF_OF_QUOTA_SIZE;
const BLENDING_HEADER_SIZE: usize = BLENDING_SUBHEADER_SIZE + PROOF_OF_SELECTION_SIZE;
const MAX_ENCAPSULATIONS: usize = 3;
const PUBLIC_HEADER_SIZE: usize = BLENDING_SUBHEADER_SIZE;
const PRIVATE_HEADER_SIZE: usize = BLENDING_HEADER_SIZE * MAX_ENCAPSULATIONS;
const PAYLOAD_HEADER_SIZE: usize = 1 + 2;
const MAX_PAYLOAD_BODY_SIZE: usize = 34 * 1024;
const PAYLOAD_SIZE: usize = PAYLOAD_HEADER_SIZE + MAX_PAYLOAD_BODY_SIZE;
const MESSAGE_SIZE: usize = HEADER_SIZE + PUBLIC_HEADER_SIZE + PRIVATE_HEADER_SIZE + PAYLOAD_SIZE;

#[derive(Debug, Clone)]
pub struct Message(BytesMut);

impl Message {
    pub fn initialize(shared_keys: &[SharedKey]) -> Self {
        if shared_keys.len() > MAX_ENCAPSULATIONS {
            panic!("Number of encapsulations exceeds the maximum allowed.");
        }

        let mut msg = BytesMut::zeroed(MESSAGE_SIZE);

        // Fill the message header.
        msg[0] = VERSION;

        // Randomize the private header.
        let private_header_pos = HEADER_SIZE + PUBLIC_HEADER_SIZE;
        for i in 0..MAX_ENCAPSULATIONS {
            let bytes = pseudo_random_bytes(
                Ed25519PrivateKey::generate().public_key().as_bytes(),
                BLENDING_HEADER_SIZE,
            );
            let offset = private_header_pos + i * BLENDING_HEADER_SIZE;
            println!(
                "Randomizing a private header at offset:{offset}, size:{BLENDING_HEADER_SIZE}"
            );
            msg[offset..offset + BLENDING_HEADER_SIZE].copy_from_slice(&bytes);
        }

        // Fill the last `shared_key.len()` blending headers with resonstructed data.
        for (i, shared_key) in shared_keys.iter().rev().enumerate() {
            let key = shared_key.as_slice();
            let r1 = pseudo_random_bytes(&concat(key, &[1]), KEY_SIZE);
            let r2 = pseudo_random_bytes(&concat(key, &[2]), PROOF_OF_QUOTA_SIZE);
            let r3 = pseudo_random_bytes(&concat(key, &[3]), SIGNATURE_SIZE);
            let r4 = pseudo_random_bytes(&concat(key, &[4]), PROOF_OF_SELECTION_SIZE);

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
            let cipher = shared_key.cipher(BLENDING_HEADER_SIZE);
            for j in 0..shared_keys.len() - i {
                let blending_header_idx = j + MAX_ENCAPSULATIONS - shared_keys.len();
                let offset = private_header_pos + blending_header_idx * BLENDING_HEADER_SIZE;
                println!("Encrypting the {blending_header_idx}-th blending header with {i}-th key: offset:{offset}, size:{BLENDING_HEADER_SIZE}");
                cipher.encrypt(&mut msg[offset..offset + BLENDING_HEADER_SIZE]);
            }
        }

        Self(msg)
    }

    pub fn encapsulate(
        self,
        shared_keys: &[SharedKey],
        signing_keys: &[(Ed25519PrivateKey, Ed25519PublicKey, ProofOfQuota)],
        proof_of_selections: &[ProofOfSelection],
        payload: &[u8],
    ) -> Self {
        if shared_keys.len() > MAX_ENCAPSULATIONS {
            panic!("Number of encapsulations exceeds the maximum allowed.");
        }
        if shared_keys.len() != signing_keys.len() {
            panic!("Number of shared keys must match the number of signing keys.");
        }
        if shared_keys.len() != proof_of_selections.len() {
            panic!("Number of shared keys must match the number of proof of selections.");
        }

        println!("===== ENCAPSULATING =====");

        let mut msg = self.0;

        let private_header_pos = HEADER_SIZE + PUBLIC_HEADER_SIZE;
        let payload_pos = private_header_pos + PRIVATE_HEADER_SIZE;
        // TODO: check payload size
        msg[payload_pos] = 0x01; // data message
        let payload_size: u16 = payload.len().try_into().unwrap();
        msg[payload_pos + 1..payload_pos + 3].copy_from_slice(&payload_size.to_le_bytes());
        msg[payload_pos + 3..payload_pos + 3 + payload.len()].copy_from_slice(payload);

        // Encapsulate the private header and payload using each shared key.
        for (i, (shared_key, proof_of_selection)) in
            shared_keys.iter().zip(proof_of_selections).enumerate()
        {
            // Sign on the concat of current private header and payload.
            let (signing_key, signing_pubkey, proof_of_quota) = if i == 0 {
                println!("Signing using the initial signing key");
                let initial_private_key = Ed25519PrivateKey::generate();
                let initial_public_key = initial_private_key.public_key();
                let initial_proof_of_quota = ProofOfQuota(
                    pseudo_random_bytes(initial_private_key.as_bytes(), PROOF_OF_QUOTA_SIZE)
                        .try_into()
                        .unwrap(),
                );
                &(
                    initial_private_key,
                    initial_public_key,
                    initial_proof_of_quota,
                )
            } else {
                println!("Signing using {i}-th signing key");
                signing_keys.get(i - 1).unwrap()
            };
            let sig = signing_key.sign(&msg[private_header_pos..]);

            // Encrypt the payload.
            println!("Encrypting payload with {i}-th shared key");
            shared_key.encrypt(&mut msg[payload_pos..]);

            // Shift blending headers by one rightward.
            println!("Shifting blending headers one rightward");
            msg.copy_within(
                private_header_pos..payload_pos - BLENDING_HEADER_SIZE,
                private_header_pos + BLENDING_HEADER_SIZE,
            );

            // Fill the first blending header.
            let mut pos = private_header_pos;
            println!("Filling the first blending header: offset:{pos}, size:{KEY_SIZE}");
            msg[pos..pos + KEY_SIZE].copy_from_slice(signing_pubkey.as_bytes());
            pos += KEY_SIZE;
            println!("Filling the first blending header: offset:{pos}, size:{PROOF_OF_QUOTA_SIZE}");
            msg[pos..pos + PROOF_OF_QUOTA_SIZE].copy_from_slice(proof_of_quota.as_bytes());
            pos += PROOF_OF_QUOTA_SIZE;
            println!("Filling the first blending header: offset:{pos}, size:{SIGNATURE_SIZE}");
            msg[pos..pos + SIGNATURE_SIZE].copy_from_slice(&sig);
            pos += SIGNATURE_SIZE;
            println!(
                "Filling the first blending header: offset:{pos}, size:{PROOF_OF_SELECTION_SIZE}"
            );
            msg[pos..pos + PROOF_OF_SELECTION_SIZE].copy_from_slice(proof_of_selection.as_bytes());

            // Encrypt the private header.
            let cipher = shared_key.cipher(BLENDING_HEADER_SIZE);
            for i in 0..MAX_ENCAPSULATIONS {
                let offset = private_header_pos + i * BLENDING_HEADER_SIZE;
                println!("Encrypting the {i}-th blending header: offset:{offset}, size:{BLENDING_HEADER_SIZE}");
                cipher.encrypt(&mut msg[offset..offset + BLENDING_HEADER_SIZE]);
            }
        }

        // Construct the public header.
        println!("Constructing the public header");
        let (signing_key, signing_pubkey, proof_of_quota) = signing_keys.last().unwrap();
        let sig = signing_key.sign(&msg[private_header_pos..]);
        let mut pos = HEADER_SIZE;
        println!("Filling the public header: offset:{pos}, size:{KEY_SIZE}");
        msg[pos..pos + KEY_SIZE].copy_from_slice(signing_pubkey.as_bytes());
        pos += KEY_SIZE;
        println!("Filling the public header: offset:{pos}, size:{PROOF_OF_QUOTA_SIZE}");
        msg[pos..pos + PROOF_OF_QUOTA_SIZE].copy_from_slice(proof_of_quota.as_bytes());
        pos += PROOF_OF_QUOTA_SIZE;
        println!("Filling the public header: offset:{pos}, size:{SIGNATURE_SIZE}");
        msg[pos..pos + SIGNATURE_SIZE].copy_from_slice(&sig);

        Self(msg)
    }

    pub fn decapsulate(
        self,
        private_key: &X25519PrivateKey,
        verify_proof_of_selection: impl Fn(&ProofOfSelection) -> bool,
    ) -> Result<Self, ()> {
        let mut msg = self.0;

        // Derive the shared key.
        let peer_public_key = Ed25519PublicKey::from_slice(
            &msg[HEADER_SIZE..HEADER_SIZE + KEY_SIZE],
        )
        .map_err(|_| {
            println!("Invalid public key in the message header.");
        })?;
        let shared_key = private_key.derive_shared_key(&peer_public_key.derive_x25519());

        // Decrypt the private header.
        let private_header_pos = HEADER_SIZE + PUBLIC_HEADER_SIZE;
        for i in 0..MAX_ENCAPSULATIONS {
            let offset = private_header_pos + i * BLENDING_HEADER_SIZE;
            println!("Decrypting the {i}-th blending header: offset:{offset}, size:{BLENDING_HEADER_SIZE}");
            shared_key.decrypt(&mut msg[offset..offset + BLENDING_HEADER_SIZE]);
        }

        // Verify the first blending header which was decrypted.
        let proof_of_selection = ProofOfSelection(
            msg[private_header_pos + KEY_SIZE + PROOF_OF_QUOTA_SIZE + SIGNATURE_SIZE
                ..private_header_pos + BLENDING_HEADER_SIZE]
                .try_into()
                .unwrap(),
        );
        if !verify_proof_of_selection(&proof_of_selection) {
            println!("Proof of selection verification failed.");
            return Err(());
        }
        // TODO: deduplication

        // Set the public header.
        let mut pos = HEADER_SIZE;
        println!("Filling the public header: offset:{pos}, size:{KEY_SIZE}");
        msg.copy_within(private_header_pos..private_header_pos + KEY_SIZE, pos);
        pos += KEY_SIZE;
        println!("Filling the public header: offset:{pos}, size:{PROOF_OF_QUOTA_SIZE}");
        msg.copy_within(
            private_header_pos + KEY_SIZE..private_header_pos + KEY_SIZE + PROOF_OF_QUOTA_SIZE,
            pos,
        );
        pos += PROOF_OF_QUOTA_SIZE;
        println!("Filling the public header: offset:{pos}, size:{SIGNATURE_SIZE}");
        msg.copy_within(
            private_header_pos + KEY_SIZE + PROOF_OF_QUOTA_SIZE
                ..private_header_pos + KEY_SIZE + PROOF_OF_QUOTA_SIZE + SIGNATURE_SIZE,
            pos,
        );

        // Decrypt the payload.
        let payload_pos = private_header_pos + PRIVATE_HEADER_SIZE;
        println!("Decrypting the payload: offset:{payload_pos}, size:{PAYLOAD_SIZE}");
        shared_key.decrypt(&mut msg[payload_pos..]);

        // Shift blending headers one leftward.
        msg.copy_within(
            private_header_pos + BLENDING_HEADER_SIZE..private_header_pos + PRIVATE_HEADER_SIZE,
            private_header_pos,
        );

        // Reconstruct the last blending header.
        let key = shared_key.as_slice();
        let r1 = pseudo_random_bytes(&concat(key, &[1]), KEY_SIZE);
        let r2 = pseudo_random_bytes(&concat(key, &[2]), PROOF_OF_QUOTA_SIZE);
        let r3 = pseudo_random_bytes(&concat(key, &[3]), SIGNATURE_SIZE);
        let r4 = pseudo_random_bytes(&concat(key, &[4]), PROOF_OF_SELECTION_SIZE);
        let last_blending_header_pos =
            private_header_pos + (MAX_ENCAPSULATIONS - 1) * BLENDING_HEADER_SIZE;
        let mut offset = last_blending_header_pos;
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

        // Encrypt the reconstructed last blending header.
        shared_key.encrypt(
            &mut msg[last_blending_header_pos..last_blending_header_pos + BLENDING_HEADER_SIZE],
        );

        // TODO: Compare signatures

        Ok(Self(msg))
    }

    pub fn payload_body(&self) -> &[u8] {
        let payload_pos = HEADER_SIZE + PUBLIC_HEADER_SIZE + PRIVATE_HEADER_SIZE;
        let size = u16::from_le_bytes(self.0[payload_pos + 1..payload_pos + 3].try_into().unwrap())
            as usize;
        &self.0[payload_pos + PAYLOAD_HEADER_SIZE..payload_pos + PAYLOAD_HEADER_SIZE + size]
    }
}

fn concat(a: &[u8], b: &[u8]) -> Vec<u8> {
    itertools::chain(a, b).copied().collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use crate::crypto::X25519PrivateKey;

    use super::*;

    #[test]
    fn initialize() {
        let keysets = generate_keysets(2);
        let msg = Message::initialize(
            &keysets
                .iter()
                .map(|k| k.shared_key.clone())
                .collect::<Vec<_>>(),
        );
        assert_eq!(msg.0.len(), MESSAGE_SIZE);
        assert_eq!(msg.0[0], VERSION);
    }

    #[test]
    fn encapsulate() {
        let keysets = generate_keysets(2);
        let initialized_msg = Message::initialize(
            &keysets
                .iter()
                .map(|k| k.shared_key.clone())
                .collect::<Vec<_>>(),
        );
        let msg = initialized_msg.clone().encapsulate(
            &keysets
                .iter()
                .map(|k| k.shared_key.clone())
                .collect::<Vec<_>>(),
            &keysets
                .iter()
                .map(|k| {
                    (
                        k.ephemeral_signing_key.clone(),
                        k.ephemeral_signing_key.public_key(),
                        ProofOfQuota([0u8; PROOF_OF_QUOTA_SIZE]),
                    )
                })
                .collect::<Vec<_>>(),
            &keysets
                .iter()
                .map(|k| ProofOfSelection(*k.nonephemeral_signing_key.public_key().as_bytes()))
                .collect::<Vec<_>>(),
            "hello".as_bytes(),
        );
        assert_eq!(msg.0.len(), MESSAGE_SIZE);
        assert_ne!(msg.0, initialized_msg.0);
    }

    #[test]
    fn decapsulate() {
        let keysets = generate_keysets(2);
        let msg = Message::initialize(
            &keysets
                .iter()
                .map(|k| k.shared_key.clone())
                .collect::<Vec<_>>(),
        );
        let msg = msg.encapsulate(
            &keysets
                .iter()
                .map(|k| k.shared_key.clone())
                .collect::<Vec<_>>(),
            &keysets
                .iter()
                .map(|k| {
                    (
                        k.ephemeral_signing_key.clone(),
                        k.ephemeral_signing_key.public_key(),
                        ProofOfQuota([0u8; PROOF_OF_QUOTA_SIZE]),
                    )
                })
                .collect::<Vec<_>>(),
            &keysets
                .iter()
                .map(|k| ProofOfSelection(*k.nonephemeral_signing_key.public_key().as_bytes()))
                .collect::<Vec<_>>(),
            "hello".as_bytes(),
        );

        // Check that the message cannot be decapsulated with the wrong ephemeral key.
        let (nonephemeral_signing_pubkey, nonephemeral_encryption_key) = (
            keysets
                .first()
                .unwrap()
                .nonephemeral_signing_key
                .public_key(),
            &keysets.first().unwrap().nonephemeral_encryption_key,
        );
        assert!(msg
            .clone()
            .decapsulate(nonephemeral_encryption_key, |proof_of_selection| {
                proof_of_selection.as_bytes() == nonephemeral_signing_pubkey.as_bytes()
            })
            .is_err());

        // Check that the message can be decapsulated with the correct ephemeral key.
        let (nonephemeral_signing_pubkey, nonephemeral_encryption_key) = (
            keysets
                .last()
                .unwrap()
                .nonephemeral_signing_key
                .public_key(),
            &keysets.last().unwrap().nonephemeral_encryption_key,
        );
        let msg = msg
            .decapsulate(nonephemeral_encryption_key, |proof_of_selection| {
                proof_of_selection.as_bytes() == nonephemeral_signing_pubkey.as_bytes()
            })
            .unwrap();

        // Check that the message can be decapsulated with the correct ephemeral key
        // and the fully-decapsulated payload is correct.
        let (nonephemeral_signing_pubkey, nonephemeral_encryption_key) = (
            keysets
                .first()
                .unwrap()
                .nonephemeral_signing_key
                .public_key(),
            &keysets.first().unwrap().nonephemeral_encryption_key,
        );
        let msg = msg
            .decapsulate(nonephemeral_encryption_key, |proof_of_selection| {
                proof_of_selection.as_bytes() == nonephemeral_signing_pubkey.as_bytes()
            })
            .unwrap();
        assert_eq!(msg.payload_body(), "hello".as_bytes());
    }

    struct KeySet {
        ephemeral_signing_key: Ed25519PrivateKey,
        ephemeral_encryption_key: X25519PrivateKey,
        nonephemeral_signing_key: Ed25519PrivateKey,
        nonephemeral_encryption_key: X25519PrivateKey,
        shared_key: SharedKey,
    }

    fn generate_keysets(cnt: usize) -> Vec<KeySet> {
        let ephemeral_keys = generate_keys(cnt);
        let nonephemeral_keys = generate_keys(cnt);

        ephemeral_keys
            .into_iter()
            .zip(nonephemeral_keys)
            .map(
                |(
                    (ephemeral_signing_key, ephemeral_encryption_key),
                    (nonephemeral_signing_key, nonephemeral_encryption_key),
                )| {
                    let shared_key = ephemeral_encryption_key
                        .derive_shared_key(&nonephemeral_encryption_key.public_key());
                    KeySet {
                        ephemeral_signing_key,
                        ephemeral_encryption_key,
                        nonephemeral_signing_key,
                        nonephemeral_encryption_key,
                        shared_key,
                    }
                },
            )
            .collect()
    }

    fn generate_keys(cnt: usize) -> Vec<(Ed25519PrivateKey, X25519PrivateKey)> {
        (0..cnt)
            .map(|_| {
                let signing_key = Ed25519PrivateKey::generate();
                let encryption_key = signing_key.derive_x25519();
                (signing_key, encryption_key)
            })
            .collect()
    }
}
