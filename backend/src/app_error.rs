//! IPC error type. SPEC.md §9/§11: every command returns Result<T, AppError>
//! with a stable machine-readable code the UI can branch on.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        AppError { code: code.into(), message: message.into() }
    }
}

impl From<tp_core::parse::ParseError> for AppError {
    fn from(e: tp_core::parse::ParseError) -> Self {
        AppError::new("parse_failed", e.to_string())
    }
}

impl From<tp_ble::BleError> for AppError {
    fn from(e: tp_ble::BleError) -> Self {
        use tp_ble::BleError::*;
        let code = match &e {
            Adapter(_) => "bt_unavailable",
            NotFound => "device_not_found",
            Incompatible(_) => "trainer_incompatible",
            ControlRefused(_) => "control_refused",
            Timeout | Disconnected => "control_lost",
            Transport(_) => "ble_transport",
        };
        AppError::new(code, e.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::new("db", e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::new("io", e.to_string())
    }
}

impl From<tp_core::journal::JournalError> for AppError {
    fn from(e: tp_core::journal::JournalError) -> Self {
        AppError::new("journal", e.to_string())
    }
}

impl From<tp_core::fit::FitError> for AppError {
    fn from(e: tp_core::fit::FitError) -> Self {
        AppError::new("fit_encode_failed", e.to_string())
    }
}
