pub mod kzgrs;
pub mod tx;

pub use nomos_core::da::DaVerifier;

pub trait VerifierBackend: DaVerifier {
    type Settings;
    fn new(settings: Self::Settings) -> Self;
}

pub trait TxVerifierBackend {
    type Settings;
    type Tx;
    type Error;
    fn new(settings: Self::Settings) -> Self;

    fn verify(&self, tx: Self::Tx) -> Result<(), Self::Error>;
}
