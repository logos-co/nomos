use std::{
    fmt::{Debug, Display, Formatter},
    io::Write,
    panic,
    sync::{Arc, Mutex},
};

use futures::StreamExt;
use nomos_tracing::{
    filter::envfilter::{create_envfilter_layer, EnvFilterConfig},
    logging::{
        gelf::{create_gelf_layer, GelfConfig},
        local::{create_file_layer, create_writer_layer, FileConfig},
        loki::{create_loki_layer, LokiConfig},
    },
    metrics::otlp::{create_otlp_metrics_layer, OtlpMetricsConfig},
    tracing::otlp::{create_otlp_tracing_layer, OtlpTracingConfig},
};
use overwatch::{
    services::{
        life_cycle::LifecycleMessage,
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceStateHandle,
};
use serde::{Deserialize, Serialize};
use tracing::{error, Level};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{filter::LevelFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub struct Tracing<RuntimeServiceId> {
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
    logger_guard: Option<WorkerGuard>,
}

/// This is a wrapper around a writer to allow cloning which is
/// required by contract by Overwatch for a configuration struct
#[derive(Clone)]
pub struct SharedWriter {
    inner: Arc<Mutex<dyn Write + Send + Sync>>,
}

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.lock().unwrap().flush()
    }
}

impl SharedWriter {
    pub fn new<W: Write + Send + Sync + 'static>(writer: W) -> Self {
        Self {
            inner: Arc::new(Mutex::new(writer)),
        }
    }

    #[must_use]
    pub fn into_inner(&self) -> Arc<Mutex<dyn Write + Send + Sync>> {
        Arc::clone(&self.inner)
    }

    pub fn from_inner(inner: Arc<Mutex<dyn Write + Send + Sync>>) -> Self {
        Self { inner }
    }
}

impl Debug for SharedWriter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedWriter").finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LoggerLayer {
    Gelf(GelfConfig),
    File(FileConfig),
    Loki(LokiConfig),
    Stdout,
    Stderr,
    #[serde(skip)]
    Writer(SharedWriter),
    // do not collect logs
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TracingLayer {
    Otlp(OtlpTracingConfig),
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FilterLayer {
    EnvFilter(EnvFilterConfig),
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MetricsLayer {
    Otlp(OtlpMetricsConfig),
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TracingSettings {
    pub logger: LoggerLayer,
    pub tracing: TracingLayer,
    pub filter: FilterLayer,
    pub metrics: MetricsLayer,
    #[serde(with = "serde_level")]
    pub level: Level,
}

impl Default for TracingSettings {
    fn default() -> Self {
        Self {
            logger: LoggerLayer::Stdout,
            tracing: TracingLayer::None,
            filter: FilterLayer::None,
            metrics: MetricsLayer::None,
            level: Level::DEBUG,
        }
    }
}

impl TracingSettings {
    #[inline]
    #[must_use]
    pub const fn new(
        logger: LoggerLayer,
        tracing: TracingLayer,
        filter: FilterLayer,
        metrics: MetricsLayer,
        level: Level,
    ) -> Self {
        Self {
            logger,
            tracing,
            filter,
            metrics,
            level,
        }
    }
}

impl<RuntimeServiceId> ServiceData for Tracing<RuntimeServiceId> {
    type Settings = TracingSettings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ();
}

#[async_trait::async_trait]
impl<RuntimeServiceId> ServiceCore<RuntimeServiceId> for Tracing<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self> + Display + Send,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        #[cfg(test)]
        use std::sync::Once;
        #[cfg(test)]
        static ONCE_INIT: Once = Once::new();

        let config = service_state.settings_reader.get_updated_settings();
        let mut layers: Vec<Box<dyn tracing_subscriber::Layer<_> + Send + Sync>> = vec![];

        let (logger_layer, logger_guard): (
            Box<dyn tracing_subscriber::Layer<_> + Send + Sync>,
            Option<WorkerGuard>,
        ) = match config.logger {
            LoggerLayer::Gelf(config) => {
                let gelf_layer =
                    create_gelf_layer(&config, service_state.overwatch_handle.runtime())?;
                (Box::new(gelf_layer), None)
            }
            LoggerLayer::File(config) => {
                let (layer, guard) = create_file_layer(config);
                (Box::new(layer), Some(guard))
            }
            LoggerLayer::Loki(config) => {
                let loki_layer =
                    create_loki_layer(config, service_state.overwatch_handle.runtime())?;
                (Box::new(loki_layer), None)
            }
            LoggerLayer::Stdout => {
                let (layer, guard) = create_writer_layer(std::io::stdout());
                (Box::new(layer), Some(guard))
            }
            LoggerLayer::Stderr => {
                let (layer, guard) = create_writer_layer(std::io::stderr());
                (Box::new(layer), Some(guard))
            }
            LoggerLayer::Writer(writer) => {
                let (layer, guard) = create_writer_layer(writer);
                (Box::new(layer), Some(guard))
            }
            LoggerLayer::None => (Box::new(tracing_subscriber::fmt::Layer::new()), None),
        };

        layers.push(logger_layer);

        if let TracingLayer::Otlp(config) = config.tracing {
            let tracing_layer = create_otlp_tracing_layer(config)?;
            layers.push(Box::new(tracing_layer));
        }

        if let FilterLayer::EnvFilter(config) = config.filter {
            let filter_layer = create_envfilter_layer(config)?;
            layers.push(Box::new(filter_layer));
        }

        if let MetricsLayer::Otlp(config) = config.metrics {
            let metrics_layer = create_otlp_metrics_layer(config)?;
            layers.push(Box::new(metrics_layer));
        }

        // If no layers are created, tracing subscriber is not required.
        if layers.is_empty() {
            return Ok(Self {
                service_state,
                logger_guard: None,
            });
        }

        #[cfg(test)]
        ONCE_INIT.call_once(move || {
            tracing_subscriber::registry()
                .with(LevelFilter::from(config.level))
                .with(layers)
                .init();
        });
        #[cfg(not(test))]
        tracing_subscriber::registry()
            .with(LevelFilter::from(config.level))
            .with(layers)
            .init();

        panic::set_hook(Box::new(nomos_tracing::panic::panic_hook));

        Ok(Self {
            service_state,
            logger_guard,
        })
    }

    async fn run(self) -> Result<(), overwatch::DynError> {
        let Self {
            service_state,
            logger_guard,
        } = self;
        // keep the handle alive without stressing the runtime
        let mut lifecycle_stream = service_state.lifecycle_handle.message_stream();
        loop {
            if let Some(msg) = lifecycle_stream.next().await {
                match msg {
                    LifecycleMessage::Shutdown(sender) => {
                        // flush pending logs before signaling message processing
                        drop(logger_guard);
                        if sender.send(()).is_err() {
                            error!(
                                "Error sending successful shutdown signal from service {}",
                                RuntimeServiceId::SERVICE_ID
                            );
                        }
                        break;
                    }
                    LifecycleMessage::Kill => {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

mod serde_level {
    use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

    use super::Level;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Level, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = <String>::deserialize(deserializer)?;
        v.parse()
            .map_err(|e| D::Error::custom(format!("invalid log level {e}")))
    }

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "Signature must match serde requirement."
    )]
    pub fn serialize<S>(value: &Level, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.as_str().serialize(serializer)
    }
}
