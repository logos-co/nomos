use std::path::PathBuf;

use nomos_tracing::logging::local::FileConfig;
use nomos_tracing_service::{
    ConsoleLayer, FilterLayer, LoggerLayer, MetricsLayer, TracingLayer, TracingSettings,
};
use tracing::Level;

use crate::IS_DEBUG_TRACING;

#[derive(Clone, Default)]
pub struct GeneralTracingConfig {
    pub tracing_settings: TracingSettings,
}

impl GeneralTracingConfig {
    fn local_debug_tracing(id: usize) -> Self {
        let host_identifier = format!("node-{id}");
        Self {
            tracing_settings: TracingSettings {
                logger: LoggerLayer::File(FileConfig {
                    directory: PathBuf::from("./logs"),
                    prefix: Some(host_identifier.into()),
                }),
                tracing: TracingLayer::None,
                filter: FilterLayer::EnvFilter(nomos_tracing::filter::envfilter::EnvFilterConfig {
                    filters: std::iter::once(&("nomos", "debug"))
                        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                        .collect(),
                }),
                metrics: MetricsLayer::None,
                console: ConsoleLayer::None,
                level: Level::DEBUG,
            },
        }
    }
}

#[must_use]
pub fn create_tracing_configs(ids: &[[u8; 32]]) -> Vec<GeneralTracingConfig> {
    if *IS_DEBUG_TRACING {
        create_debug_configs(ids)
    } else {
        create_default_configs(ids)
    }
}

fn create_debug_configs(ids: &[[u8; 32]]) -> Vec<GeneralTracingConfig> {
    ids.iter()
        .enumerate()
        .map(|(i, _)| GeneralTracingConfig::local_debug_tracing(i))
        .collect()
}

fn create_default_configs(ids: &[[u8; 32]]) -> Vec<GeneralTracingConfig> {
    ids.iter()
        .map(|_| GeneralTracingConfig {
            tracing_settings: TracingSettings {
                logger: LoggerLayer::File(FileConfig {
                    directory: PathBuf::from("./logs"),
                    prefix: Some(PathBuf::from("nomos-node")),
                }),
                tracing: TracingLayer::None,
                filter: FilterLayer::None,
                metrics: MetricsLayer::None,
                console: ConsoleLayer::None,
                level: Level::INFO,
            },
        })
        .collect()
}
