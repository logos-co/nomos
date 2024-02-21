pub mod select;

use crate::da::blob::Blob;
use bytes::Bytes;
use std::hash::Hash;

pub trait Certificate {
    type Blob: Blob;
    type Hash: Hash + Eq + Clone;
    fn blob(&self) -> <Self::Blob as Blob>::Hash;
    fn hash(&self) -> Self::Hash;
    fn as_bytes(&self) -> Bytes;
}

pub trait BlobCertificateSelect {
    type Certificate: Certificate;
    type Settings: Clone;

    fn new(settings: Self::Settings) -> Self;
    fn select_blob_from<'i, I: Iterator<Item = Self::Certificate> + 'i>(
        &self,
        certificates: I,
    ) -> impl Iterator<Item = Self::Certificate> + 'i;
}

pub trait CertificateStrategy {
    type Attestation;
    type Certificate;

    fn can_build(&self, attestations: &[Self::Attestation]) -> bool;
    fn build(&self, attestations: Vec<Self::Attestation>) -> Self::Certificate;
}
