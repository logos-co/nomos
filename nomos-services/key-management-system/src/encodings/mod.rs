mod errors;

pub use errors::EncodingError;

#[expect(dead_code, reason = "Will be used when integrating KMS.")]
pub enum DataEncoding {
    Ed25519(bytes::Bytes),
}

#[expect(dead_code, reason = "Will be used when integrating KMS.")]
pub enum SignatureEncoding {
    Ed25519(ed25519_dalek::Signature),
}

#[expect(dead_code, reason = "Will be used when integrating KMS.")]
pub enum PublicKeyEncoding {
    Ed25519(ed25519_dalek::VerifyingKey),
}
