//! Utilities for representing and working with `Service`s' status information.
//!
//! This module provides types that help display and convert
//! [`ServiceStatus`](overwatch::services::status::ServiceStatus) data.

pub mod entry;
pub mod error;
mod status_wrapper;

pub use entry::ServiceStatusEntry;
pub use error::ServiceStatusEntriesError;
