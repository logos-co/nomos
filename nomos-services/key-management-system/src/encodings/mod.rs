mod errors;

pub use errors::EncodingError;

/// Entity that gathers all payload encodings required by the KMS crate.
#[expect(dead_code, reason = "Will be used when integrating KMS.")]
pub enum PayloadEncoding {
    Ed25519(bytes::Bytes),
}

/// Entity that gathers all signature encodings required by the KMS crate.
#[expect(dead_code, reason = "Will be used when integrating KMS.")]
pub enum SignatureEncoding {
    Ed25519(ed25519_dalek::Signature),
}

/// Entity that gathers all public key encodings required by the KMS crate.
#[expect(dead_code, reason = "Will be used when integrating KMS.")]
pub enum PublicKeyEncoding {
    Ed25519(ed25519_dalek::VerifyingKey),
}
