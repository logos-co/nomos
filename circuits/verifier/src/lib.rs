mod rapidsnark;
mod traits;
mod wrappers;

pub use rapidsnark::Rapidsnark;
pub use traits::Verifier;

pub type Result<T> = std::io::Result<T>;
