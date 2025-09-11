use crate::settings::AxumBackendSettings;

type DefaultGovernorLayer = tower_governor::GovernorLayer<
    'static,
    tower_governor::key_extractor::PeerIpKeyExtractor,
    governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>,
>;

/// Create a `GovernorLayer` for rate limiting based on the settings
#[must_use]
pub fn create_rate_limit_layer(settings: &AxumBackendSettings) -> DefaultGovernorLayer {
    tower_governor::GovernorLayer {
        config: Box::leak(Box::new(
            tower_governor::governor::GovernorConfigBuilder::default()
                .per_second(settings.rate_limit_per_second)
                .burst_size(settings.rate_limit_burst)
                .finish()
                .expect("Failed to create governor config"),
        )),
    }
}
