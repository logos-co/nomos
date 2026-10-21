//! Provides conversion from a collection of [`ServiceStatusEntry`] to
//! the general-purpose [`DynError`](overwatch::DynError) type.

use std::fmt::{Debug, Display, Formatter};

use crate::overwatch::status::ServiceStatusEntry;

#[derive(Debug)]
pub struct ServiceStatusEntriesError<RuntimeServiceId: Display> {
    service_status_entries: Vec<ServiceStatusEntry<RuntimeServiceId>>,
}

impl<RuntimeServiceId: Display> Display for ServiceStatusEntriesError<RuntimeServiceId> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let entries: Vec<String> = self
            .service_status_entries
            .iter()
            .map(ToString::to_string)
            .collect();
        let entries = entries.join(", ");
        write!(f, "ServiceStatuses: {entries}")
    }
}

impl<RuntimeServiceId: Display + Debug> std::error::Error
    for ServiceStatusEntriesError<RuntimeServiceId>
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.service_status_entries
            .iter()
            .find_map(ServiceStatusEntry::wait_error)
            .map(|wait_error| wait_error as &(dyn std::error::Error + 'static))
    }
}

impl<RuntimeServiceId, Iter> From<Iter> for ServiceStatusEntriesError<RuntimeServiceId>
where
    RuntimeServiceId: Display,
    Iter: IntoIterator<Item = ServiceStatusEntry<RuntimeServiceId>>,
{
    fn from(entries: Iter) -> Self {
        Self {
            service_status_entries: entries.into_iter().collect(),
        }
    }
}
