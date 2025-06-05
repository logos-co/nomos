use std::fmt::{Debug, Display, Formatter};

pub struct ServiceStatusEntry<RuntimeServiceId> {
    id: RuntimeServiceId,
    status: status_wrapper::ServiceStatus,
}

impl<RuntimeServiceId> ServiceStatusEntry<RuntimeServiceId> {
    pub const fn from_overwatch(
        id: RuntimeServiceId,
        status: overwatch::services::status::ServiceStatus,
    ) -> Self {
        Self {
            id,
            status: status_wrapper::ServiceStatus::new(status),
        }
    }
}

impl<RuntimeServiceId: Display> Display for ServiceStatusEntry<RuntimeServiceId> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let Self { id, status } = self;
        let id = id.to_string();
        write!(f, "{id}: {status}")
    }
}

impl<RuntimeServiceId: Display> Debug for ServiceStatusEntry<RuntimeServiceId> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let Self { id, status } = self;
        let id = id.to_string();
        write!(f, "ServiceStatusEntry {{ id: {id}, status: {status:?} }}")
    }
}

impl<RuntimeServiceId: Display> std::error::Error for ServiceStatusEntry<RuntimeServiceId> {}

mod status_wrapper {
    //! Temporary wrapper to implement `Display` for
    //! `overwatch::services::status::ServiceStatus` until the `overwatch`
    //! crate provides it.

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
}

pub(crate) mod errors {
    //! Provides conversion from a collection of [`ServiceStatusEntry`] to
    //! the general [`DynError`](overwatch::DynError) type.

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
}
