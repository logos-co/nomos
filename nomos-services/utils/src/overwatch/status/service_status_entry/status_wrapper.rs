//! Provides a temporary wrapper that provides a [`Display`] implementation for
//! [`ServiceStatus`](overwatch::services::status::ServiceStatus).
//!
//! This is a stopgap until the [`overwatch`] crate natively implements
//! `Display` for `ServiceStatus`. It allows user-friendly formatting and error
//! reporting in the meantime.

use std::fmt::{Debug, Display, Formatter};

pub struct ServiceStatus(overwatch::services::status::ServiceStatus);

impl ServiceStatus {
    pub const fn new(status: overwatch::services::status::ServiceStatus) -> Self {
        Self(status)
    }
}

impl Display for ServiceStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use overwatch::services::status::ServiceStatus::{Ready, Starting, Stopped};

        let display_value = match self.0 {
            Starting => "Starting",
            Ready => "Ready",
            Stopped => "Stopped",
        };

        write!(f, "{display_value}")
    }
}

impl Debug for ServiceStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}
