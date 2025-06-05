use std::fmt::{Debug, Display, Formatter};

use serde::ser::StdError;

struct ServiceStatus(overwatch::services::status::ServiceStatus);

impl Display for ServiceStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use overwatch::services::status::ServiceStatus::*;

        let display_value = match self.0 {
            Starting => "Starting",
            Ready => "Ready",
            Stopped => "Stopped",
        };

        write!(f, "{}", display_value)
    }
}

impl Debug for ServiceStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

pub struct ServiceStatusEntry {
    id: String,
    status: ServiceStatus,
}

impl Display for ServiceStatusEntry {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let Self { id, status } = self;
        write!(f, "{}: {}", id, status)
    }
}

impl Debug for ServiceStatusEntry {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let Self { id, status } = self;
        write!(
            f,
            "ServiceStatusEntry {{ id: {}, status: {:?} }}",
            id, status
        )
    }
}

impl StdError for ServiceStatusEntry {}
