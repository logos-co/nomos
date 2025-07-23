use crate::header::HeaderId;

/// A block trait to abstract complex block structures,
/// especially for testing purposes.
pub trait Block {
    fn id(&self) -> HeaderId;
}
