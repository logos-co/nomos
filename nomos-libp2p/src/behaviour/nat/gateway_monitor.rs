use std::{
    net::IpAddr,
    pin::Pin,
    task::{Context, Poll},
};

use futures::FutureExt as _;
use tracing::{debug, info, warn};

use crate::config::GatewaySettings;

/// Events emitted by the gateway monitor
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayMonitorEvent {
    /// Gateway address has changed
    GatewayChanged {
        /// Previous gateway address
        old_gateway: Option<IpAddr>,
        /// New gateway address
        new_gateway: IpAddr,
    },
}

pub trait GatewayDetector: Send + Sync {
    /// Detect the current gateway address
    fn detect() -> Result<IpAddr, String>;
}

/// System gateway detector that uses the OS's routing table to find the default
/// gateway.
///
/// This detector queries the system's network configuration to identify the
/// current default gateway IP address, which is typically the router or NAT
/// device that provides internet connectivity.
pub struct SystemGatewayDetector;

impl GatewayDetector for SystemGatewayDetector {
    fn detect() -> Result<IpAddr, String> {
        default_net::get_default_gateway()
            .map(|gateway| gateway.ip_addr)
            .map_err(|e| format!("Failed to get default gateway: {e}"))
    }
}

/// Gateway monitoring behavior that periodically checks for gateway changes
pub struct GatewayMonitor<Detector: GatewayDetector> {
    /// Configuration settings
    settings: GatewaySettings,
    /// Current gateway address (if known)
    current_gateway: Option<IpAddr>,
    /// Timer for periodic gateway checks
    check_timer: Pin<Box<tokio::time::Sleep>>,
    _detector: std::marker::PhantomData<Detector>,
}

impl<Detector: GatewayDetector> GatewayMonitor<Detector> {
    pub fn new(settings: GatewaySettings) -> Self {
        let check_timer = Box::pin(tokio::time::sleep(settings.check_interval));

        let mut monitor = Self {
            settings,
            current_gateway: None,
            check_timer,
            _detector: std::marker::PhantomData,
        };

        debug!(
            "Starting gateway monitoring with {:?}s interval",
            monitor.settings.check_interval
        );

        monitor.perform_initial_check();

        monitor
    }

    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Option<GatewayMonitorEvent>> {
        if self.check_timer.as_mut().poll_unpin(cx).is_ready() {
            let event = self.check_gateway();

            self.check_timer = Box::pin(tokio::time::sleep(self.settings.check_interval));

            if event.is_some() {
                return Poll::Ready(event);
            }
        }

        Poll::Pending
    }

    /// Run the initial gateway check
    fn perform_initial_check(&mut self) {
        match Detector::detect() {
            Ok(gateway) => {
                info!("Initial gateway detected: {gateway}");
                self.current_gateway = Some(gateway);
            }
            Err(e) => {
                warn!("Failed to detect initial gateway: {e}");
            }
        }
    }

    fn check_gateway(&mut self) -> Option<GatewayMonitorEvent> {
        match Detector::detect() {
            Ok(new_gateway) => match self.current_gateway {
                Some(old_gateway) if old_gateway != new_gateway => {
                    info!("Gateway address changed from {old_gateway} to {new_gateway}");
                    self.current_gateway = Some(new_gateway);

                    Some(GatewayMonitorEvent::GatewayChanged {
                        old_gateway: Some(old_gateway),
                        new_gateway,
                    })
                }
                _ => None,
            },
            Err(e) => {
                warn!("Failed to detect gateway: {e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use tokio::time::timeout;

    use super::*;

    struct MockDetector;

    thread_local! {
        static CHANGE_COUNTER: AtomicUsize = const { AtomicUsize::new(0) };
    }

    impl GatewayDetector for MockDetector {
        fn detect() -> Result<IpAddr, String> {
            let count = CHANGE_COUNTER.with(|c| c.fetch_add(1, Ordering::SeqCst));
            if count == 0 {
                Ok("192.168.1.1".parse().unwrap())
            } else {
                Ok("192.168.1.254".parse().unwrap())
            }
        }
    }

    #[tokio::test]
    async fn test_gateway_changes() {
        tokio::time::pause();

        let mut monitor = GatewayMonitor::<MockDetector>::new(GatewaySettings::default());

        assert_eq!(
            monitor.current_gateway,
            Some("192.168.1.1".parse().unwrap())
        );

        tokio::time::advance(GatewaySettings::default().check_interval).await;

        timeout(Duration::from_secs(10), async {
            loop {
                if let Some(GatewayMonitorEvent::GatewayChanged { .. }) =
                    futures::future::poll_fn(|cx| monitor.poll(cx)).await
                {
                    return;
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(
            monitor.current_gateway,
            Some("192.168.1.254".parse().unwrap())
        );
    }
}
