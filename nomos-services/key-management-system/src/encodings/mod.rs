mod errors;

pub use errors::EncodingError;

pub enum DataEncoding {
    #[expect(dead_code, reason = "Will be used when adding the ZK key.")]
    Ed25519(bytes::Bytes),
}

pub enum SignatureEncoding {
    #[expect(dead_code, reason = "Will be used when integrating KMS.")]
    Ed25519(ed25519_dalek::Signature),
}

pub enum PublicKeyEncoding {
    #[expect(dead_code, reason = "Will be used when integrating KMS.")]
    Ed25519(ed25519_dalek::VerifyingKey),
}
