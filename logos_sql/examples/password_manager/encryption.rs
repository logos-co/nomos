//! Encrypt fields before SQL sees them. The key never enters the database.

use std::{fs::File, io::Read as _, path::Path};

use chacha20poly1305::{
    XChaCha20Poly1305,
    aead::{Aead as _, AeadCore as _, KeyInit as _, Payload},
};
use rand::rngs::OsRng;
use zeroize::Zeroizing;

use crate::{AppError, AppResult};

const NONCE_BYTES: usize = 24;

pub struct Encryption {
    cipher: XChaCha20Poly1305,
    channel: String,
}

impl Encryption {
    pub fn from_file(path: &Path, channel: &[u8]) -> AppResult<Self> {
        let file = File::open(path)?;

        let mut key = Zeroizing::new(Vec::new());
        file.take(33).read_to_end(&mut key)?;

        Self::new(&key, channel)
    }

    fn new(key: &[u8], channel: &[u8]) -> AppResult<Self> {
        let cipher =
            XChaCha20Poly1305::new_from_slice(key).map_err(|_| AppError::InvalidEncryptionKey)?;

        Ok(Self {
            cipher,
            channel: hex::encode(channel),
        })
    }

    pub fn encrypt(&self, label: &str, field: &str, value: &str) -> AppResult<Vec<u8>> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);

        let context = self.context(label, field);
        let encrypted = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: value.as_bytes(),
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| AppError::EncryptionFailed)?;

        // Each stored value carries its own nonce so another replica can decrypt it.
        let mut stored = nonce.to_vec();
        stored.extend(encrypted);

        Ok(stored)
    }

    pub fn decrypt(&self, label: &str, field: &str, stored: &[u8]) -> AppResult<String> {
        let (nonce, encrypted) = stored
            .split_first_chunk::<NONCE_BYTES>()
            .ok_or(AppError::DecryptionFailed)?;
        let context = self.context(label, field);
        let plaintext = self
            .cipher
            .decrypt(
                nonce.into(),
                Payload {
                    msg: encrypted,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| AppError::DecryptionFailed)?;

        String::from_utf8(plaintext).map_err(|_| AppError::DecryptionFailed)
    }

    fn context(&self, label: &str, field: &str) -> String {
        // Binding ciphertext to its channel, field and label prevents moving it
        // to another entry or interpreting encrypted notes as a password.
        format!(
            "logos-sql-password-manager:v1:{}:{field}:{label}",
            self.channel
        )
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn key_file_can_be_reopened_but_must_have_the_right_length() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&[1; 32]).unwrap();
        let writer = Encryption::from_file(file.path(), &[2; 32]).unwrap();
        let stored = writer.encrypt("email", "notes", "").unwrap();
        let reader = Encryption::from_file(file.path(), &[2; 32]).unwrap();

        assert_eq!(reader.decrypt("email", "notes", &stored).unwrap(), "");

        file.write_all(&[0]).unwrap();
        assert!(matches!(
            Encryption::from_file(file.path(), &[2; 32]),
            Err(AppError::InvalidEncryptionKey)
        ));
        file.as_file().set_len(31).unwrap();
        assert!(matches!(
            Encryption::from_file(file.path(), &[2; 32]),
            Err(AppError::InvalidEncryptionKey)
        ));
    }

    #[test]
    fn replicas_decrypt_fields_but_reject_the_wrong_key_or_context() {
        let writer = Encryption::new(&[1; 32], &[2; 32]).unwrap();
        let reader = Encryption::new(&[1; 32], &[2; 32]).unwrap();
        let stored = writer.encrypt("email", "password", "secret").unwrap();

        assert_eq!(
            reader.decrypt("email", "password", &stored).unwrap(),
            "secret"
        );
        assert_ne!(
            stored,
            writer.encrypt("email", "password", "secret").unwrap()
        );
        assert!(reader.decrypt("other", "password", &stored).is_err());
        assert!(reader.decrypt("email", "notes", &stored).is_err());
        assert!(
            Encryption::new(&[3; 32], &[2; 32])
                .unwrap()
                .decrypt("email", "password", &stored)
                .is_err()
        );
        assert!(
            Encryption::new(&[1; 32], &[3; 32])
                .unwrap()
                .decrypt("email", "password", &stored)
                .is_err()
        );

        let mut corrupted = stored;
        corrupted[NONCE_BYTES] ^= 1;
        assert!(reader.decrypt("email", "password", &corrupted).is_err());
        assert!(reader.decrypt("email", "password", &[0; 24]).is_err());
        assert!(reader.decrypt("email", "password", &[0; 23]).is_err());
    }
}
