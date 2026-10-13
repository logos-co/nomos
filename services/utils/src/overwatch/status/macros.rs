/// Macro call that waits for services to be in `ServiceStatus::Ready` state.
/// It can wait from 1..n services to be ready.
///
/// # Arguments
///
/// Requires `RuntimeServiceId` to be defined in the current scope.
///
/// * `$overwatch_handle`: A reference to the `OverwatchHandle` that manages the
///   services.
/// * `$timeout`: An optional `Duration` that specifies how long to wait for the
///   services to be ready. If `None`, it will wait indefinitely.
/// * `$( $service_type:ident ),+`: A list of service types that should be
///   checked for readiness.
///
/// # Returns
///
/// An async scope that returns:
/// - `Ok(())` if all specified services are ready within the timeout.
/// - `Err(DynError)` if any of the specified services are not ready within the
///   timeout.
///
/// # Example
///
/// ```rust,ignore
/// use std::time::Duration;
/// use overwatch::{DynError, overwatch::OverwatchHandle};
/// use lb_services_utils::overwatch::status::wait_until_services_are_ready;
///
/// // The following types would be defined as part of your Overwatch runtime.
/// struct RuntimeServiceId;
/// struct ServiceA;
/// struct ServiceB;
/// struct ServiceC;
///
/// // Mock function to get an OverwatchHandle
/// fn get_overwatch_handle() -> OverwatchHandle<RuntimeServiceId> {
///     unimplemented!()
/// }
///
/// async fn main() {
///     let overwatch_handle = get_overwatch_handle();
///     let _: Result<(), DynError> = wait_until_services_are_ready!(
///        &overwatch_handle,
///        Some(Duration::from_secs(10)),
///        ServiceA,
///        ServiceB,
///        ServiceC
///     ).await;
/// }
/// ```
///
/// A complete example with working service implementations is available in this
/// module's tests.
#[macro_export]
macro_rules! wait_until_services_are_ready {
    ( $overwatch_handle:expr, $timeout:expr, $( $service_type:ty ),+ ) => {
        async {
            let overwatch_handle: &::overwatch::overwatch::OverwatchHandle<RuntimeServiceId> = $overwatch_handle;
            let timeout: Option<::std::time::Duration> = $timeout;
            let mut wait_for_futures: Vec<::std::pin::Pin<Box<dyn ::std::future::Future<Output = ::std::result::Result::<(), $crate::overwatch::status::ServiceStatusEntry::<RuntimeServiceId>>> + Send>>> = Vec::new();

            // Iterate over each service type and create a future to wait for its readiness
            $(
                let mut status_watcher = overwatch_handle.status_watcher::<$service_type>().await.map_err(
                  ::overwatch::DynError::from
                )?;
                let wait_for_future = async move {
                    if let Err(service_status) = status_watcher
                        .wait_for(::overwatch::services::status::ServiceStatus::Ready, timeout)
                        .await
                    {
                        let service_id = <RuntimeServiceId as ::overwatch::services::AsServiceId<$service_type>>::SERVICE_ID;
                        let service_status_entry = $crate::overwatch::status::ServiceStatusEntry::<RuntimeServiceId>::from_overwatch(service_id, service_status);
                        return Err(service_status_entry);
                    }

                    ::std::result::Result::<(), $crate::overwatch::status::ServiceStatusEntry::<RuntimeServiceId>>::Ok(())
                };
                let pinned_wait_for_future = Box::pin(wait_for_future);
                wait_for_futures.push(pinned_wait_for_future);
            )+

            // Wait for all futures to complete
            let results: Vec<::std::result::Result<(), $crate::overwatch::status::ServiceStatusEntry::<RuntimeServiceId>>> = ::futures::future::join_all(wait_for_futures).await;

            // Filter out any errors from the results
            let errors: Vec<$crate::overwatch::status::ServiceStatusEntry::<RuntimeServiceId>> = results.into_iter().filter_map(|res| res.err()).collect();

            // If any of the services are not ready, return an error with the service status entries
            if !errors.is_empty() {
                let error: $crate::overwatch::status::ServiceStatusEntriesError<RuntimeServiceId> = errors.into();
                return ::std::result::Result::Err(::overwatch::DynError::from(error));
            }

            // If all services are ready, return Ok(())
            ::std::result::Result::<(), ::overwatch::DynError>::Ok(())
        }
    };
}

pub use wait_until_services_are_ready;

/// Wait for multiple dependencies to become ready while observing terminal
/// service lifecycle states.
///
/// Unlike [`wait_until_services_are_ready`], this macro reports both
/// `Stopped` and `Failed` as soon as they are observed and composes the waits
/// fail-fast. Every dependency must have passed an acknowledged startup
/// boundary before this macro is called. Overwatch's status watcher exposes
/// only the latest status, so `Stopped` is terminal even if it is the initial
/// status observed by the watcher. The node startup sequence satisfies this
/// prerequisite: it awaits `start_service_sequence`, which starts services
/// sequentially before acknowledging the sequence.
///
/// The wait has no readiness deadline. Periodic progress logs do not terminate
/// it, and dropping the returned future cancels all waits.
#[macro_export]
macro_rules! wait_until_services_are_ready_with_lifecycle {
    ( $overwatch_handle:expr, $( $service_type:ty ),+ ) => {{
        async {
            let overwatch_handle: &::overwatch::overwatch::OverwatchHandle<RuntimeServiceId> = $overwatch_handle;
            let mut wait_for_futures = Vec::new();

            $(
                let status_watcher = overwatch_handle
                    .status_watcher::<$service_type>()
                    .await
                    .map_err(::overwatch::DynError::from)?;
                let service_id =
                    <RuntimeServiceId as ::overwatch::services::AsServiceId<$service_type>>::SERVICE_ID;
                wait_for_futures.push(
                    $crate::overwatch::status::wait_for_service_to_be_ready(
                        status_watcher,
                        service_id,
                    ),
                );
            )+

            if let Err(service_status_entry) =
                ::futures::future::try_join_all(wait_for_futures).await
            {
                let error: $crate::overwatch::status::ServiceStatusEntriesError<RuntimeServiceId> =
                    [service_status_entry].into();
                return ::std::result::Result::Err(::overwatch::DynError::from(error));
            }

            ::std::result::Result::<(), ::overwatch::DynError>::Ok(())
        }
    }};
}

pub use wait_until_services_are_ready_with_lifecycle;

#[cfg(test)]
mod tests {
    use std::{
        fmt::{Debug, Display},
        sync::Arc,
        time::Duration,
    };

    use async_trait::async_trait;
    use overwatch::{
        DynError, OpaqueServiceResourcesHandle,
        overwatch::{Overwatch, OverwatchRunner, Shutdown},
        services::{
            AsServiceId, ServiceCore, ServiceData,
            state::{NoOperator, NoState},
            status::ServiceStatus,
        },
    };
    use overwatch_derive::derive_services;
    use tokio::{runtime::Handle, sync::Notify};

    async fn notify_ready_and_wait<Service: ServiceData, RuntimeServiceId>(
        service_resources_handle: &OpaqueServiceResourcesHandle<Service, RuntimeServiceId>,
    ) where
        Service::Message: Send,
        Service::State: Send + Sync,
        Service::Settings: Send + Sync,
        RuntimeServiceId: Send,
    {
        // Notify that the service is ready
        service_resources_handle.status_updater.notify_ready();

        // Await infinitely so Status::Ready can be observed
        std::future::pending::<()>().await;
    }

    struct LightService {
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    }

    impl ServiceData for LightService {
        type Settings = ();
        type State = NoState<Self::Settings>;
        type StateOperator = NoOperator<Self::State>;
        type Message = ();
    }

    #[async_trait]
    impl ServiceCore<RuntimeServiceId> for LightService {
        fn init(
            service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
            _initial_state: Self::State,
        ) -> Result<Self, DynError> {
            Ok(Self {
                service_resources_handle,
            })
        }

        async fn run(self) -> Result<(), DynError> {
            notify_ready_and_wait::<Self, RuntimeServiceId>(&self.service_resources_handle).await;
            Ok(())
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum HeavyServiceOutcome {
        Ready,
        ReturnError,
        Panic,
    }

    #[derive(Clone, Debug)]
    struct HeavyServiceSettings {
        startup_delay: Duration,
        outcome: HeavyServiceOutcome,
        started: Option<Arc<Notify>>,
    }

    impl HeavyServiceSettings {
        fn ready_after(startup_delay: Duration) -> Self {
            Self {
                startup_delay,
                outcome: HeavyServiceOutcome::Ready,
                started: None,
            }
        }
    }

    struct HeavyService {
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        settings: HeavyServiceSettings,
    }

    impl ServiceData for HeavyService {
        type Settings = HeavyServiceSettings;
        type State = NoState<Self::Settings>;
        type StateOperator = NoOperator<Self::State>;
        type Message = ();
    }

    #[async_trait]
    impl ServiceCore<RuntimeServiceId> for HeavyService {
        fn init(
            service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
            _initial_state: Self::State,
        ) -> Result<Self, DynError> {
            let settings = service_resources_handle
                .settings_handle
                .notifier()
                .get_updated_settings();
            Ok(Self {
                service_resources_handle,
                settings,
            })
        }

        async fn run(self) -> Result<(), DynError> {
            if let Some(started) = &self.settings.started {
                started.notify_one();
            }
            tokio::time::sleep(self.settings.startup_delay).await;

            match self.settings.outcome {
                HeavyServiceOutcome::Ready => {
                    notify_ready_and_wait::<Self, RuntimeServiceId>(&self.service_resources_handle)
                        .await;
                    Ok(())
                }
                HeavyServiceOutcome::ReturnError => {
                    Err(DynError::from("mock dependency startup error"))
                }
                HeavyServiceOutcome::Panic => panic!("mock dependency startup panic"),
            }
        }
    }

    struct LifecycleDependantService<Dependency, RuntimeServiceId> {
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _dependency: std::marker::PhantomData<Dependency>,
    }

    impl<Dependency, RuntimeServiceId> ServiceData
        for LifecycleDependantService<Dependency, RuntimeServiceId>
    {
        type Settings = ();
        type State = NoState<Self::Settings>;
        type StateOperator = NoOperator<Self::State>;
        type Message = ();
    }

    #[async_trait]
    impl<Dependency, RuntimeServiceId> ServiceCore<RuntimeServiceId>
        for LifecycleDependantService<Dependency, RuntimeServiceId>
    where
        RuntimeServiceId:
            Debug + Send + Sync + Display + AsServiceId<Dependency> + AsServiceId<Self> + 'static,
        Dependency: Send + Sync + 'static,
    {
        fn init(
            service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
            _initial_state: Self::State,
        ) -> Result<Self, DynError> {
            Ok(Self {
                service_resources_handle,
                _dependency: std::marker::PhantomData,
            })
        }

        async fn run(self) -> Result<(), DynError> {
            wait_until_services_are_ready_with_lifecycle!(
                &self.service_resources_handle.overwatch_handle,
                Dependency
            )
            .await?;
            notify_ready_and_wait::<Self, RuntimeServiceId>(&self.service_resources_handle).await;
            Ok(())
        }
    }

    struct NestedGenericService<GenericService, RuntimeServiceId> {
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    }

    impl<GenericService, RuntimeServiceId> ServiceData
        for NestedGenericService<GenericService, RuntimeServiceId>
    {
        type Settings = ();
        type State = NoState<Self::Settings>;
        type StateOperator = NoOperator<Self::State>;
        type Message = ();
    }

    #[async_trait]
    impl<GenericService, RuntimeServiceId> ServiceCore<RuntimeServiceId>
        for NestedGenericService<GenericService, RuntimeServiceId>
    where
        RuntimeServiceId: Debug + Send + Sync + Display + AsServiceId<GenericService> + 'static,
    {
        fn init(
            service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
            _initial_state: Self::State,
        ) -> Result<Self, DynError> {
            Ok(Self {
                service_resources_handle,
            })
        }

        async fn run(self) -> Result<(), DynError> {
            wait_until_services_are_ready!(
                &self.service_resources_handle.overwatch_handle,
                None,
                GenericService
            )
            .await?;
            notify_ready_and_wait::<Self, RuntimeServiceId>(&self.service_resources_handle).await;
            Ok(())
        }
    }

    struct DependantService<GenericService, RuntimeServiceId> {
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    }

    impl<GenericService, RuntimeServiceId> ServiceData
        for DependantService<GenericService, RuntimeServiceId>
    {
        type Settings = ();
        type State = NoState<Self::Settings>;
        type StateOperator = NoOperator<Self::State>;
        type Message = ();
    }

    #[async_trait]
    impl<GenericService, RuntimeServiceId> ServiceCore<RuntimeServiceId>
        for DependantService<GenericService, RuntimeServiceId>
    where
        RuntimeServiceId: Debug
            + Send
            + Sync
            + Display
            + AsServiceId<GenericService>
            + AsServiceId<HeavyService>
            + AsServiceId<Self>
            + 'static,
    {
        fn init(
            service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
            _initial_state: Self::State,
        ) -> Result<Self, DynError> {
            Ok(Self {
                service_resources_handle,
            })
        }

        async fn run(self) -> Result<(), DynError> {
            wait_until_services_are_ready!(
                &self.service_resources_handle.overwatch_handle,
                None,
                GenericService,
                HeavyService
            )
            .await?;
            notify_ready_and_wait::<Self, RuntimeServiceId>(&self.service_resources_handle).await;
            Ok(())
        }
    }

    #[derive_services(panic_policy = Option<Shutdown>)]
    struct App {
        light_service: LightService,
        lifecycle_dependant_service: LifecycleDependantService<HeavyService, RuntimeServiceId>,
        heavy_service: HeavyService,
        nested_generic_service: NestedGenericService<LightService, RuntimeServiceId>,
        dependent_service: DependantService<
            NestedGenericService<LightService, RuntimeServiceId>,
            RuntimeServiceId,
        >,
    }

    fn initialize() -> Overwatch<RuntimeServiceId> {
        initialize_with(
            HeavyServiceSettings::ready_after(Duration::from_secs(2)),
            None,
        )
    }

    fn initialize_with(
        heavy_service: HeavyServiceSettings,
        runtime_handle: Option<Handle>,
    ) -> Overwatch<RuntimeServiceId> {
        let settings = AppServiceSettings {
            light_service: (),
            lifecycle_dependant_service: (),
            heavy_service,
            nested_generic_service: (),
            dependent_service: (),
        };
        OverwatchRunner::<App>::run(settings, runtime_handle).expect("Failed to run overwatch")
    }

    #[test]
    fn test_wait_until_services_are_ready_macro() {
        let overwatch = initialize();
        let overwatch_handle = overwatch.handle();
        drop(
            overwatch_handle
                .runtime()
                .block_on(overwatch_handle.start_all_services()),
        );

        // Wait until ServiceC is ready, which depends on ServiceA and ServiceB
        let mut status_watcher = overwatch_handle.runtime().block_on(async {
            overwatch_handle
                .status_watcher::<DependantService<
                    NestedGenericService<LightService, RuntimeServiceId>,
                    RuntimeServiceId,
                >>()
                .await
                .expect("Status watcher should be produced.")
        });

        let dependent_service_status = overwatch_handle.runtime().block_on(async {
            status_watcher
                .wait_for(ServiceStatus::Ready, Some(Duration::from_secs(5)))
                .await
        });

        assert_eq!(
            dependent_service_status,
            Ok(ServiceStatus::Ready),
            "DependentService should be Ready."
        );

        drop(
            overwatch_handle
                .runtime()
                .block_on(overwatch_handle.shutdown()),
        );
    }

    #[test]
    fn test_wait_until_services_are_ready_macro_timeout() {
        let overwatch = initialize();
        let overwatch_handle = overwatch.handle();
        drop(
            overwatch_handle
                .runtime()
                .block_on(overwatch_handle.start_all_services()),
        );

        // Wait for a service that will not be ready, expecting a timeout error
        let mut status_watcher = overwatch_handle.runtime().block_on(async {
            overwatch_handle
                .status_watcher::<DependantService<
                    NestedGenericService<LightService, RuntimeServiceId>,
                    RuntimeServiceId,
                >>()
                .await
                .expect("Status watcher should be produced.")
        });

        let dependent_service_status = overwatch_handle.runtime().block_on(async {
            status_watcher
                .wait_for(ServiceStatus::Ready, Some(Duration::from_secs(1)))
                .await
        });

        assert_eq!(
            dependent_service_status,
            Err(ServiceStatus::Starting),
            "DependantService should still be Starting."
        );

        // Teardown
        drop(
            overwatch_handle
                .runtime()
                .block_on(overwatch_handle.shutdown()),
        );
        overwatch
            .blocking_wait_finished()
            .expect("No service should panic");
    }

    #[tokio::test(start_paused = true)]
    async fn lifecycle_wait_allows_slow_dependency_and_start_order_race() {
        let started = Arc::new(Notify::new());
        let mut settings = HeavyServiceSettings::ready_after(Duration::from_secs(61));
        settings.started = Some(Arc::clone(&started));
        let overwatch = initialize_with(settings, Some(Handle::current()));
        let handle = overwatch.handle();

        // The lifecycle dependant is first in the service sequence, while its
        // dependency is still initially Stopped. Overwatch handles its status
        // watcher request only after the full sequential start command, by
        // which time HeavyService has entered Starting.
        handle
            .start_all_services()
            .await
            .expect("services should start");
        started.notified().await;

        let dependency_status = handle
            .status_watcher::<HeavyService>()
            .await
            .expect("dependency status watcher should be available");
        let mut dependent_status = handle
            .status_watcher::<LifecycleDependantService<HeavyService, RuntimeServiceId>>()
            .await
            .expect("dependent status watcher should be available");

        let mut readiness = Box::pin(wait_until_services_are_ready_with_lifecycle!(
            &handle,
            HeavyService
        ));
        tokio::select! {
            biased;
            result = &mut readiness => panic!("dependency completed before the delay: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert!(futures::poll!(readiness.as_mut()).is_pending());
        assert_eq!(
            dependency_status.current(),
            ServiceStatus::Starting,
            "dependency should still be Starting beyond the former deadline"
        );
        assert_eq!(
            dependent_status.current(),
            ServiceStatus::Starting,
            "dependent should wait rather than fail before dependency readiness"
        );

        tokio::time::advance(Duration::from_secs(1)).await;
        readiness
            .await
            .expect("slow dependency should become ready");
        assert_eq!(
            dependent_status
                .wait_for(ServiceStatus::Ready, Some(Duration::from_secs(5)))
                .await,
            Ok(ServiceStatus::Ready),
            "dependent should become Ready after its dependency"
        );

        handle.shutdown().await.expect("Overwatch should shut down");
        overwatch
            .wait_finished()
            .await
            .expect("No service should panic");
    }

    #[tokio::test(start_paused = true)]
    async fn lifecycle_wait_reports_stopped_before_watcher_attachment() {
        let mut settings = HeavyServiceSettings::ready_after(Duration::ZERO);
        settings.outcome = HeavyServiceOutcome::ReturnError;
        let overwatch = initialize_with(settings, Some(Handle::current()));
        let handle = overwatch.handle();

        handle
            .start_service::<HeavyService>()
            .await
            .expect("dependency should start");
        let mut watcher = handle
            .status_watcher::<HeavyService>()
            .await
            .expect("status watcher should be available");
        assert_eq!(
            watcher
                .wait_for(ServiceStatus::Stopped, Some(Duration::from_secs(5)))
                .await,
            Ok(ServiceStatus::Stopped),
            "mock dependency should stop before the lifecycle wait attaches"
        );

        let error = wait_until_services_are_ready_with_lifecycle!(&handle, HeavyService)
            .await
            .expect_err("Stopped dependency must fail readiness");
        let message = error.to_string();
        let service_id = <RuntimeServiceId as AsServiceId<HeavyService>>::SERVICE_ID;
        assert!(message.contains(&service_id.to_string()));
        assert!(message.contains("Stopped"));

        handle.shutdown().await.expect("Overwatch should shut down");
        overwatch
            .wait_finished()
            .await
            .expect("No service should panic");
    }

    #[tokio::test(start_paused = true)]
    async fn lifecycle_wait_reports_panicked_dependency() {
        let mut settings = HeavyServiceSettings::ready_after(Duration::from_secs(1));
        settings.outcome = HeavyServiceOutcome::Panic;
        let overwatch = initialize_with(settings, Some(Handle::current()));
        let handle = overwatch.handle();

        handle
            .start_service::<HeavyService>()
            .await
            .expect("dependency should start");
        let mut readiness = Box::pin(wait_until_services_are_ready_with_lifecycle!(
            &handle,
            HeavyService
        ));
        tokio::select! {
            biased;
            result = &mut readiness => panic!("dependency completed before the panic: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
        tokio::time::advance(Duration::from_secs(1)).await;

        let error = readiness
            .await
            .expect_err("Failed dependency must fail readiness");
        let message = error.to_string();
        let service_id = <RuntimeServiceId as AsServiceId<HeavyService>>::SERVICE_ID;
        assert!(message.contains(&service_id.to_string()));
        assert!(message.contains("Failed"));

        handle.shutdown().await.expect("Overwatch should shut down");
        overwatch
            .wait_finished()
            .await
            .expect("No service should panic");
    }
}
