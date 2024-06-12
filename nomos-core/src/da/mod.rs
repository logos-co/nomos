use std::error::Error;
// crates
// internal

pub mod attestation;
pub mod certificate;
pub mod certificate_metadata;
pub mod vid;

pub trait DaEncoder {
    type EncodedData;
    fn encode(b: &[u8]) -> Result<Self::EncodedData, impl Error>;
}

pub trait DaVerifier {
    type DaBlob;
    type Attestation;
    fn verify(&self, blob: Self::DaBlob) -> Result<Self::Attestation, impl Error>;
}

pub trait DaDispersal {
    type EncodedData;
    type Certificate;

    fn disperse(&self, encoded_data: Self::EncodedData) -> Result<Self::Certificate, impl Error>;
}
