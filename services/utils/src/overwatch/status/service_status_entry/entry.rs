//! Defines the [`ServiceStatusEntry`] type, which associates a service
//! identifier with its current [`ServiceStatus`].
//!
//! This module is useful for tracking the readiness or operational state of
//! services within an Overwatch-based system. It provides an ergonomic way to
//! collect and display service statuses, particularly when aggregating
//! diagnostics or errors.

use std::{
    error::Error,
    fmt::{Debug, Display, Formatter},
};

use overwatch::services::status::{ServiceStatus, WaitForError};

pub struct ServiceStatusEntry<RuntimeServiceId> {
    id: RuntimeServiceId,
    status: ServiceStatus,
    wait_error: Option<WaitForError>,
}

impl<RuntimeServiceId> ServiceStatusEntry<RuntimeServiceId> {
    pub const fn from_overwatch(id: RuntimeServiceId, status: ServiceStatus) -> Self {
        Self {
            id,
            status,
            wait_error: None,
        }
    }

    pub const fn from_wait_error(id: RuntimeServiceId, wait_error: WaitForError) -> Self {
        let status = match wait_error {
            WaitForError::ServiceStopped { .. } => ServiceStatus::Stopped,
            WaitForError::ServiceFailed { .. } => ServiceStatus::Failed,
            WaitForError::Timeout {
                latest_status: status,
                ..
            }
            | WaitForError::ChannelClosed {
                latest_status: status,
                ..
            } => status,
        };

        Self {
            id,
            status,
            wait_error: Some(wait_error),
        }
    }

    /// Returns the structured Overwatch readiness error, if this entry came
    /// from a failed readiness wait.
    pub const fn wait_error(&self) -> Option<&WaitForError> {
        self.wait_error.as_ref()
    }
}

impl<RuntimeServiceId: Display> Display for ServiceStatusEntry<RuntimeServiceId> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let Self {
            id,
            status,
            wait_error,
        } = self;
        let id = id.to_string();
        match wait_error {
            Some(WaitForError::ServiceStopped { initial_status }) => {
                write!(
                    f,
                    "Dependency stopped before readiness ({id}: initial {initial_status}, observed {})",
                    ServiceStatus::Stopped
                )
            }
            Some(WaitForError::ServiceFailed { initial_status }) => {
                write!(
                    f,
                    "Dependency failed before readiness ({id}: initial {initial_status}, observed {})",
                    ServiceStatus::Failed
                )
            }
            Some(WaitForError::Timeout {
                initial_status,
                latest_status,
            }) => {
                write!(
                    f,
                    "Dependency readiness deadline expired ({id}: initial {initial_status}, latest {latest_status})"
                )
            }
            Some(WaitForError::ChannelClosed {
                initial_status,
                latest_status,
            }) => {
                write!(
                    f,
                    "Dependency status notifications closed before readiness ({id}: initial {initial_status}, latest {latest_status})"
                )
            }
            None => write!(f, "{id}: {status}"),
        }
    }
}

impl<RuntimeServiceId: Display> Debug for ServiceStatusEntry<RuntimeServiceId> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let Self {
            id,
            status,
            wait_error,
        } = self;
        let id = id.to_string();
        write!(
            f,
            "ServiceStatusEntry {{ id: {id}, status: {status:?}, wait_error: {wait_error:?} }}"
        )
    }
}

impl<RuntimeServiceId: Display> Error for ServiceStatusEntry<RuntimeServiceId> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.wait_error
            .as_ref()
            .map(|wait_error| wait_error as &(dyn Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_only_entries_keep_their_existing_display() {
        let entry = ServiceStatusEntry::from_overwatch("Wallet", ServiceStatus::Ready);

        assert_eq!(entry.to_string(), "Wallet: ServiceStatus::Ready");
        assert!(Error::source(&entry).is_none());
    }

    #[test]
    fn readiness_errors_keep_initial_and_observed_statuses_and_source() {
        for (wait_error, expected_initial, expected_observed) in [
            (
                WaitForError::ServiceStopped {
                    initial_status: ServiceStatus::Ready,
                },
                "initial ServiceStatus::Ready",
                "observed ServiceStatus::Stopped",
            ),
            (
                WaitForError::ServiceFailed {
                    initial_status: ServiceStatus::Starting,
                },
                "initial ServiceStatus::Starting",
                "observed ServiceStatus::Failed",
            ),
            (
                WaitForError::Timeout {
                    initial_status: ServiceStatus::Stopped,
                    latest_status: ServiceStatus::Starting,
                },
                "initial ServiceStatus::Stopped",
                "latest ServiceStatus::Starting",
            ),
            (
                WaitForError::ChannelClosed {
                    initial_status: ServiceStatus::Failed,
                    latest_status: ServiceStatus::Ready,
                },
                "initial ServiceStatus::Failed",
                "latest ServiceStatus::Ready",
            ),
        ] {
            let entry = ServiceStatusEntry::from_wait_error("Wallet", wait_error);
            let message = entry.to_string();

            assert!(message.contains("Wallet"));
            assert!(message.contains(expected_initial));
            assert!(message.contains(expected_observed));
            assert_eq!(
                Error::source(&entry).and_then(|source| source.downcast_ref::<WaitForError>()),
                Some(&wait_error)
            );
        }
    }
}
