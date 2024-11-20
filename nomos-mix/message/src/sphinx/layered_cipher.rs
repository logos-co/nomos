use std::marker::PhantomData;

use rand_chacha::{
    rand_core::{RngCore, SeedableRng},
    ChaCha12Rng,
};
use sphinx_packet::{
    constants::HEADER_INTEGRITY_MAC_SIZE,
    crypto::STREAM_CIPHER_INIT_VECTOR,
    header::{
        keys::{HeaderIntegrityMacKey, StreamCipherKey},
        mac::HeaderIntegrityMac,
    },
};

use super::parse_bytes;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Invalid cipher text length")]
    InvalidCipherTextLength,
    #[error("Invalid encryption param")]
    InvalidEncryptionParam,
    #[error("Integrity MAC verification failed")]
    IntegrityMacVerificationFailed,
}

type Result<T> = std::result::Result<T, Error>;

/// A cipher to encrypt/decrypt a list of data of the same size using a list of keys.
///
/// The cipher performs the layered encryption.
/// The following example shows the simplified output.
/// - Input: [[data0, k0], [data1, k1]]
/// - Output: encrypt(k0, [data0, encrypt(k1, [data1])])
///
/// The max number of layers is limited to the `max_layers` parameter.
/// Even if the number of data and keys provided for encryption is smaller than `max_layers`,
/// The cipher always produces the max-size output regardless of the number of data and keys provided,
/// in order to ensure that all outputs generated by the cipher are the same size.
///
/// The cipher also provides the length-preserved decryption.
/// Even if one layer of encryptions is decrypted, the length of decrypted data is
/// the same as the length of the original data.
/// For example:
///   len(encrypt(k0, [data0, encrypt(k1, [data1])])) == len(encrypt(k1, [data1]))
pub struct ConsistentLengthLayeredCipher<D> {
    /// All encrypted data produced by the cipher has the same size according to the `max_layers`.
    pub max_layers: usize,
    _data: PhantomData<D>,
}

pub trait ConsistentLengthLayeredCipherData {
    // Returns the serialized bytes for an instance of the implementing type
    fn to_bytes(&self) -> Vec<u8>;
    // Returns the size of the serialized data. This is a static method.
    fn size() -> usize;
}

/// A parameter for one layer of encryption
pub struct EncryptionParam<D: ConsistentLengthLayeredCipherData> {
    /// A data to be included in the layer.
    pub data: D,
    /// A [`Key`] to encrypt the layer that will include the [`Self::data`].
    pub key: Key,
}

/// A set of keys to encrypt/decrypt a single layer.
pub struct Key {
    /// A 128-bit key for encryption/decryption
    pub stream_cipher_key: StreamCipherKey,
    /// A 128-bit key for computing/verifying integrity MAC
    pub integrity_mac_key: HeaderIntegrityMacKey,
}

impl<D: ConsistentLengthLayeredCipherData> ConsistentLengthLayeredCipher<D> {
    pub fn new(max_layers: usize) -> Self {
        Self {
            max_layers,
            _data: Default::default(),
        }
    }

    /// The total size of fully encrypted output that includes all layers.
    /// This size is determined by [`D::size`] and [`Self::max_layers`].
    pub fn total_size(&self) -> usize {
        Self::single_layer_size() * self.max_layers
    }

    /// The size of a single layer that contains a data and a MAC.
    /// The MAC is used to verify integrity of the encrypted next layer.
    fn single_layer_size() -> usize {
        D::size() + HEADER_INTEGRITY_MAC_SIZE
    }

    /// Perform the layered encryption.
    pub fn encrypt(&self, params: &[EncryptionParam<D>]) -> Result<(Vec<u8>, HeaderIntegrityMac)> {
        if params.is_empty() || params.len() > self.max_layers {
            return Err(Error::InvalidEncryptionParam);
        }

        params
            .iter()
            .take(params.len() - 1) // Exclude the last param that will be treated separately below.
            .rev() // Data and keys must be used in reverse order to encrypt the inner-most layer first
            .try_fold(self.build_last_layer(params)?, |(encrypted, mac), param| {
                self.build_intermediate_layer(param, mac, encrypted)
            })
    }

    /// Build an intermediate layer of encryption that wraps subsequent layers already encrypted.
    /// The output has the same size as [`Self::total_size`],
    /// regardless of how many subsequent layers that this layer wraps.
    fn build_intermediate_layer(
        &self,
        param: &EncryptionParam<D>,
        next_mac: HeaderIntegrityMac,
        next_encrypted_data: Vec<u8>,
    ) -> Result<(Vec<u8>, HeaderIntegrityMac)> {
        // Concatenate the data with the encrypted subsequent layers and its MAC.
        let data = param.data.to_bytes();
        let total_data = itertools::chain!(
            &data,
            next_mac.as_bytes(),
            // Truncate last bytes for the length-preserved decryption later.
            // They will be restored by a filler during the decryption process.
            &next_encrypted_data[..next_encrypted_data.len() - Self::single_layer_size()],
        )
        .copied()
        .collect::<Vec<_>>();

        // Encrypt the concatenated bytes, and compute MAC.
        let mut encrypted = total_data;
        self.apply_streamcipher(
            &mut encrypted,
            &param.key.stream_cipher_key,
            StreamCipherOption::FromFront,
        );
        let mac = Self::compute_mac(&param.key.integrity_mac_key, &encrypted);

        assert_eq!(encrypted.len(), self.total_size());
        Ok((encrypted, mac))
    }

    /// Build the last layer of encryption.
    /// The output has the same size as [`Self::total_size`] by using fillers,
    /// even though it doesn't wrap any subsequent layer.
    /// This is for the length-preserved decryption.
    fn build_last_layer(
        &self,
        params: &[EncryptionParam<D>],
    ) -> Result<(Vec<u8>, HeaderIntegrityMac)> {
        let last_param = params.last().ok_or(Error::InvalidEncryptionParam)?;

        // Build fillers that will be appended to the last data.
        // The number of fillers must be the same as the number of intermediate layers
        // (excluding the last layer) that will be decrypted later.
        let fillers = self.build_fillers(&params[..params.len() - 1]);
        // Header integrity MAC doesn't need to be included in the last layer
        // because there is no next encrypted layer.
        // Instead, random bytes are used to fill the space between data and fillers.
        // The size of random bytes depends on the [`self.max_layers`].
        let random_bytes = random_bytes(self.total_size() - D::size() - fillers.len());

        // First, concat the data and the random bytes, and encrypt it.
        let last_data = last_param.data.to_bytes();
        let total_data_without_fillers = itertools::chain!(&last_data, &random_bytes)
            .copied()
            .collect::<Vec<_>>();
        let mut encrypted = total_data_without_fillers;
        self.apply_streamcipher(
            &mut encrypted,
            &last_param.key.stream_cipher_key,
            StreamCipherOption::FromFront,
        );

        // Append fillers to the encrypted bytes, and compute MAC.
        encrypted.extend(fillers);
        let mac = Self::compute_mac(&last_param.key.integrity_mac_key, &encrypted);

        assert_eq!(encrypted.len(), self.total_size());
        Ok((encrypted, mac))
    }

    /// Build as many fillers as the number of keys provided.
    /// Fillers are encrypted in accumulated manner by keys.
    fn build_fillers(&self, params: &[EncryptionParam<D>]) -> Vec<u8> {
        let single_layer_size = Self::single_layer_size();
        let mut fillers = vec![0u8; single_layer_size * params.len()];
        params
            .iter()
            .map(|param| &param.key.stream_cipher_key)
            .enumerate()
            .for_each(|(i, key)| {
                self.apply_streamcipher(
                    &mut fillers[0..(i + 1) * single_layer_size],
                    key,
                    StreamCipherOption::FromBack,
                )
            });
        fillers
    }

    /// Unpack one layer of encryption by performing the length-preserved decryption.
    pub fn unpack(
        &self,
        mac: &HeaderIntegrityMac,
        encrypted_total_data: &[u8],
        key: &Key,
    ) -> Result<(Vec<u8>, HeaderIntegrityMac, Vec<u8>)> {
        if encrypted_total_data.len() != self.total_size() {
            return Err(Error::InvalidCipherTextLength);
        }
        // If a wrong key is used, the decryption should fail.
        if !mac.verify(key.integrity_mac_key, encrypted_total_data) {
            return Err(Error::IntegrityMacVerificationFailed);
        }

        // Extend the encrypted data by the length of a single layer
        // in order to restore the truncated part (a encrypted filler)
        // by [`Self::build_intermediate_layer`] during the encryption process.
        let total_data_with_zero_filler = encrypted_total_data
            .iter()
            .copied()
            .chain(std::iter::repeat(0u8).take(Self::single_layer_size()))
            .collect::<Vec<_>>();

        // Decrypt the extended data.
        let mut decrypted = total_data_with_zero_filler;
        self.apply_streamcipher(
            &mut decrypted,
            &key.stream_cipher_key,
            StreamCipherOption::FromFront,
        );

        // Parse the decrypted data into 3 parts: data, MAC, and the next encrypted data.
        let parsed = parse_bytes(
            &decrypted,
            &[D::size(), HEADER_INTEGRITY_MAC_SIZE, self.total_size()],
        )
        .unwrap();
        let data = parsed[0].to_vec();
        let next_mac = HeaderIntegrityMac::from_bytes(parsed[1].try_into().unwrap());
        let next_encrypted_data = parsed[2].to_vec();
        Ok((data, next_mac, next_encrypted_data))
    }

    fn apply_streamcipher(&self, data: &mut [u8], key: &StreamCipherKey, opt: StreamCipherOption) {
        let pseudorandom_bytes = sphinx_packet::crypto::generate_pseudorandom_bytes(
            key,
            &STREAM_CIPHER_INIT_VECTOR,
            self.total_size() + Self::single_layer_size(),
        );
        let pseudorandom_bytes = match opt {
            StreamCipherOption::FromFront => &pseudorandom_bytes[..data.len()],
            StreamCipherOption::FromBack => {
                &pseudorandom_bytes[pseudorandom_bytes.len() - data.len()..]
            }
        };
        Self::xor_in_place(data, pseudorandom_bytes)
    }

    // In-place XOR operation: b is applied to a.
    fn xor_in_place(a: &mut [u8], b: &[u8]) {
        assert_eq!(a.len(), b.len());
        a.iter_mut().zip(b.iter()).for_each(|(x1, &x2)| *x1 ^= x2);
    }

    fn compute_mac(key: &HeaderIntegrityMacKey, data: &[u8]) -> HeaderIntegrityMac {
        let mac = sphinx_packet::crypto::compute_keyed_hmac::<sha2::Sha256>(key, data).into_bytes();
        assert!(mac.len() >= HEADER_INTEGRITY_MAC_SIZE);
        HeaderIntegrityMac::from_bytes(
            mac.into_iter()
                .take(HEADER_INTEGRITY_MAC_SIZE)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        )
    }
}

fn random_bytes(size: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; size];
    let mut rng = ChaCha12Rng::from_entropy();
    rng.fill_bytes(&mut bytes);
    bytes
}

enum StreamCipherOption {
    FromFront,
    FromBack,
}

#[cfg(test)]
mod tests {
    use sphinx_packet::{constants::INTEGRITY_MAC_KEY_SIZE, crypto::STREAM_CIPHER_KEY_SIZE};

    use super::*;

    #[test]
    fn build_and_unpack() {
        let cipher = ConsistentLengthLayeredCipher::<[u8; 10]>::new(5);

        let params = (0u8..3)
            .map(|i| EncryptionParam::<[u8; 10]> {
                data: [i; 10],
                key: Key {
                    stream_cipher_key: [i * 10; STREAM_CIPHER_KEY_SIZE],
                    integrity_mac_key: [i * 20; INTEGRITY_MAC_KEY_SIZE],
                },
            })
            .collect::<Vec<_>>();

        let (encrypted, mac) = cipher.encrypt(&params).unwrap();

        let next_encrypted = encrypted.clone();
        let (data, next_mac, next_encrypted) = cipher
            .unpack(&mac, &next_encrypted, &params[0].key)
            .unwrap();
        assert_eq!(data, params[0].data);
        assert_eq!(next_encrypted.len(), encrypted.len());

        let (data, next_mac, next_encrypted) = cipher
            .unpack(&next_mac, &next_encrypted, &params[1].key)
            .unwrap();
        assert_eq!(data, params[1].data);
        assert_eq!(next_encrypted.len(), encrypted.len());

        let (data, _, next_encrypted) = cipher
            .unpack(&next_mac, &next_encrypted, &params[2].key)
            .unwrap();
        assert_eq!(data, params[2].data);
        assert_eq!(next_encrypted.len(), encrypted.len());
    }

    impl ConsistentLengthLayeredCipherData for [u8; 10] {
        fn to_bytes(&self) -> Vec<u8> {
            self.to_vec()
        }

        fn size() -> usize {
            10
        }
    }
}
