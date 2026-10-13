use std::{fmt::Display, time::Duration};

use lb_log_targets::utils;
use overwatch::services::status::{ServiceStatus, StatusWatcher};
use tokio::time::{Instant, MissedTickBehavior, interval_at};
use tracing::{error, info};

use crate::overwatch::status::ServiceStatusEntry;

const LOG_TARGET: &str = utils::SERVICE_READINESS;
const PROGRESS_INTERVAL: Duration = Duration::from_secs(60);
const LONG_WAIT: Duration = Duration::from_secs(5);

/// Wait for a service to become ready, or report a terminal lifecycle state.
///
/// Call this only after the dependency has passed an acknowledged startup
/// boundary. `StatusWatcher` exposes only the latest state, so an initial
/// `Stopped` status is treated as terminal just like any later `Stopped`.
///
/// This waits on the service's existing status notifications, without imposing
/// a startup deadline. The progress timer only emits diagnostics. The future
/// is polled inline, so dropping its caller cancels the wait without leaving a
/// detached monitoring task.
#[expect(
    clippy::cognitive_complexity,
    reason = "The lifecycle states and notification/progress select form one readiness state machine."
)]
pub async fn wait_for_service_to_be_ready<RuntimeServiceId>(
    mut status_watcher: StatusWatcher,
    service_id: RuntimeServiceId,
) -> Result<(), ServiceStatusEntry<RuntimeServiceId>>
where
    RuntimeServiceId: Display,
{
    let started_at = Instant::now();
    let mut progress = interval_at(started_at + PROGRESS_INTERVAL, PROGRESS_INTERVAL);
    progress.set_missed_tick_behavior(MissedTickBehavior::Skip);

    info!(
        target: LOG_TARGET,
        dependency = %service_id,
        "Waiting for dependency readiness"
    );

    loop {
        let status = status_watcher.current();
        match status {
            ServiceStatus::Ready => {
                let elapsed = started_at.elapsed();
                if elapsed >= LONG_WAIT {
                    info!(
                        target: LOG_TARGET,
                        dependency = %service_id,
                        elapsed_seconds = elapsed.as_secs_f64(),
                        "Dependency became ready"
                    );
                }
                return Ok(());
            }
            ServiceStatus::Starting => {}
            ServiceStatus::Stopped => {
                error!(
                    target: LOG_TARGET,
                    dependency = %service_id,
                    status = %status,
                    elapsed_seconds = started_at.elapsed().as_secs_f64(),
                    "Dependency stopped before becoming ready"
                );
                return Err(ServiceStatusEntry::from_overwatch(service_id, status));
            }
            ServiceStatus::Failed => {
                error!(
                    target: LOG_TARGET,
                    dependency = %service_id,
                    status = %status,
                    elapsed_seconds = started_at.elapsed().as_secs_f64(),
                    "Dependency failed before becoming ready"
                );
                return Err(ServiceStatusEntry::from_overwatch(service_id, status));
            }
        }

        tokio::select! {
            changed = status_watcher.receiver_mut().changed() => {
                if changed.is_err() {
                    let status = status_watcher.current();
                    error!(
                        target: LOG_TARGET,
                        dependency = %service_id,
                        status = %status,
                        elapsed_seconds = started_at.elapsed().as_secs_f64(),
                        "Dependency status notifications closed before readiness"
                    );
                    return Err(ServiceStatusEntry::from_overwatch(service_id, status));
                }
            }
            _ = progress.tick() => {
                info!(
                    target: LOG_TARGET,
                    dependency = %service_id,
                    status = %status,
                    elapsed_seconds = started_at.elapsed().as_secs_f64(),
                    "Dependency is still starting"
                );
            }
        }
    }
}
