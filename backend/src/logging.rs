//! Normal per-launch application tracing, including the frontend bridge.

use std::path::Path;

use tracing_subscriber::prelude::*;

const FRONTEND_TRACE_MAX_BYTES: usize = 1_024;
const SESSION_LOG_FILE: &str = "trainerpro.log";

pub fn init(log_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(log_dir)?;
    let log_file = std::fs::File::create(log_dir.join(SESSION_LOG_FILE))?;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,tp_ble=debug".into());
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(log_file)),
        )
        .try_init()?;
    tracing::info!("event=backend_ready");
    Ok(())
}

/// Frontend events use logfmt and deliberately omit transcripts, workout or
/// measurement data, device identity, and paths.
#[tauri::command]
pub fn trace_frontend(message: String) {
    if message.len() <= FRONTEND_TRACE_MAX_BYTES
        && !message.chars().any(|character| character.is_control())
    {
        tracing::info!(target: "trainerpro_frontend", "{message}");
    }
}
