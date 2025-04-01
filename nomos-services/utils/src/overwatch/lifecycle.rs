use std::fmt::Display;

use overwatch::services::{life_cycle::LifecycleMessage, AsServiceId};
use tracing::{debug, error};

/// Handles the shutdown signal from `Overwatch`
pub fn should_stop_service<Service, RuntimeServiceId>(msg: &LifecycleMessage) -> bool
where
    RuntimeServiceId: AsServiceId<Service> + Display,
{
    match msg {
        LifecycleMessage::Shutdown(sender) => {
            if sender.send(()).is_err() {
                error!(
                    "Error sending successful shutdown signal from service {}",
                    RuntimeServiceId::SERVICE_ID
                );
            }
            debug!(
                "{} {}",
                RuntimeServiceId::SERVICE_ID,
                "Shutting down service"
            );
            true
        }
        LifecycleMessage::Kill => {
            debug!("{} {}", RuntimeServiceId::SERVICE_ID, "Killing service");
            true
        }
    }
}
