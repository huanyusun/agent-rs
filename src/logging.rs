use crate::{config::AppConfig, error::Result};
use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, EnvFilter};

pub fn init_logging(config: &AppConfig) -> Result<WorkerGuard> {
    let state_dir = config
        .app
        .active_workspace_file
        .parent()
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(state_dir)?;
    let file_appender = tracing_appender::rolling::never(state_dir, "research-harness.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(std::env::var("RESEARCH_HARNESS_LOG").unwrap_or_default()))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .ok();

    Ok(guard)
}
