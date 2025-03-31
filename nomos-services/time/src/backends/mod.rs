mod common;
#[cfg(feature = "ntp")]
pub mod ntp;
pub mod system_time;

#[cfg(feature = "ntp")]
pub use ntp::{NtpTimeBackend, NtpTimeBackendSettings};
pub use system_time::{SystemTimeBackend, SystemTimeBackendSettings};

use crate::EpochSlotTickStream;

/// Abstraction over slot ticking systems
pub trait TimeBackend {
    type Settings;
    fn init(settings: Self::Settings) -> Self;
    fn tick_stream(self) -> EpochSlotTickStream;
}
