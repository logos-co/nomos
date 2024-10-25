// std
use std::{io::Write, path::PathBuf};
// crates
use serde::{Deserialize, Serialize};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::{
    format::{DefaultFields, Format},
    Layer,
};
// internal

pub type FmtLayer<S> = Layer<S, DefaultFields, Format, tracing_appender::non_blocking::NonBlocking>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileConfig {
    pub directory: PathBuf,
    pub prefix: Option<PathBuf>,
}

pub fn create_file_layer<S>(config: FileConfig) -> (FmtLayer<S>, WorkerGuard) {
    let file_appender = tracing_appender::rolling::hourly(
        config.directory,
        config.prefix.unwrap_or_else(|| PathBuf::from("nomos.log")),
    );

    create_writer_layer(file_appender)
}

pub fn create_writer_layer<S, W>(writer: W) -> (FmtLayer<S>, WorkerGuard)
where
    W: Write + Send + 'static,
{
    let (non_blocking, guard) = tracing_appender::non_blocking(writer);

    let layer = tracing_subscriber::fmt::Layer::new()
        .with_level(true)
        .with_writer(non_blocking);

    (layer, guard)
}
