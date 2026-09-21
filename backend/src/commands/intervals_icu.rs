//! Intervals.icu connection lifecycle and bounded schedule refresh commands.

use serde::Serialize;
use tauri::State;

use crate::app_error::AppError;
use crate::app_state::{now_unix_ms, AppState};
use crate::database::{provider_connections, scheduled_workouts};
use crate::intervals_icu::{IntervalsApiError, IntervalsIcuClient, PROVIDER_ID};
use crate::intervals_icu_sync::{self, IntervalsSyncError};
use crate::local_date;

const SYNC_LOOKBACK_DAYS: i64 = 7;
const SYNC_LOOKAHEAD_DAYS: i64 = 42;
const CREDENTIAL_USERNAME_PREFIX: &str = "intervals_icu:";

#[derive(Debug, Clone, Serialize)]
pub struct IntervalsConnectionStatus {
    pub connected: bool,
    pub external_account_id: Option<String>,
    pub display_name: Option<String>,
    pub time_zone: Option<String>,
    pub last_sync_succeeded_at_unix_ms: Option<i64>,
    pub last_sync_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntervalsSyncReport {
    pub inserted: u32,
    pub updated: u32,
    pub unchanged: u32,
    pub removed: u32,
    pub unsupported: u32,
    pub issues: Vec<String>,
    pub oldest_date_local: String,
    pub newest_date_local: String,
}

fn disconnected_status() -> IntervalsConnectionStatus {
    IntervalsConnectionStatus {
        connected: false,
        external_account_id: None,
        display_name: None,
        time_zone: None,
        last_sync_succeeded_at_unix_ms: None,
        last_sync_error: None,
    }
}

fn status_from_row(row: provider_connections::ProviderConnectionRow) -> IntervalsConnectionStatus {
    IntervalsConnectionStatus {
        connected: row.disconnected_at_unix_ms.is_none(),
        external_account_id: Some(row.external_account_id),
        display_name: row.display_name,
        time_zone: Some(row.time_zone),
        last_sync_succeeded_at_unix_ms: row.last_sync_succeeded_at_unix_ms,
        last_sync_error: row.last_sync_error,
    }
}

#[tauri::command]
pub async fn get_intervals_icu_connection(
    state: State<'_, AppState>,
) -> Result<IntervalsConnectionStatus, AppError> {
    let connection =
        provider_connections::get_active_by_provider(&state.db.lock().unwrap(), PROVIDER_ID)?;
    Ok(connection
        .map(status_from_row)
        .unwrap_or_else(disconnected_status))
}

#[tauri::command]
pub async fn connect_intervals_icu(
    state: State<'_, AppState>,
    api_key: String,
) -> Result<IntervalsConnectionStatus, AppError> {
    let api_key = api_key.trim().to_string();
    let athlete = IntervalsIcuClient::new()
        .map_err(api_error)?
        .fetch_athlete(&api_key)
        .await
        .map_err(api_error)?;
    let now = now_unix_ms() as i64;

    let connection_id = {
        let conn = state.db.lock().unwrap();
        if let Some(active) = provider_connections::get_active_by_provider(&conn, PROVIDER_ID)? {
            if active.external_account_id != athlete.id {
                return Err(AppError::new(
                    "intervals_account_conflict",
                    "Disconnect the current Intervals.icu account before connecting another one",
                ));
            }
        }
        provider_connections::get_by_provider_account(&conn, PROVIDER_ID, &athlete.id)?
            .map(|row| row.id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    };

    store_api_key(
        state.credential_service.clone(),
        connection_id.clone(),
        api_key,
    )
    .await?;

    let connected_id = {
        let conn = state.db.lock().unwrap();
        provider_connections::connect(
            &conn,
            &provider_connections::ConnectedProvider {
                id: &connection_id,
                provider: PROVIDER_ID,
                external_account_id: &athlete.id,
                display_name: athlete.display_name(),
                time_zone: athlete.timezone.trim(),
                connected_at_unix_ms: now,
            },
        )?
    };
    let row = provider_connections::get(&state.db.lock().unwrap(), &connected_id)?
        .ok_or_else(|| AppError::new("db", "connected provider row was not found"))?;
    Ok(status_from_row(row))
}

#[tauri::command]
pub async fn refresh_intervals_icu(
    state: State<'_, AppState>,
    today_date_local: String,
) -> Result<IntervalsSyncReport, AppError> {
    let (oldest_date_local, newest_date_local) = sync_window(&today_date_local)?;
    let connection =
        provider_connections::get_active_by_provider(&state.db.lock().unwrap(), PROVIDER_ID)?
            .ok_or_else(|| {
                AppError::new(
                    "intervals_not_connected",
                    "Connect Intervals.icu in Settings before synchronizing workouts",
                )
            })?;
    let api_key = match load_api_key(state.credential_service.clone(), connection.id.clone()).await
    {
        Ok(api_key) => api_key,
        Err(error) => {
            record_sync_failure(&state, &connection.id, &error.message);
            return Err(error);
        }
    };
    let client = IntervalsIcuClient::new().map_err(api_error)?;
    let events = match client
        .fetch_workout_events(&api_key, &oldest_date_local, &newest_date_local)
        .await
    {
        Ok(events) => events,
        Err(error) => {
            record_sync_failure(&state, &connection.id, &error.to_string());
            return Err(api_error(error));
        }
    };
    let synced_at_unix_ms = now_unix_ms() as i64;
    let summary = match intervals_icu_sync::persist_calendar_window(
        &mut state.db.lock().unwrap(),
        &connection.id,
        &oldest_date_local,
        &newest_date_local,
        &events,
        synced_at_unix_ms,
    ) {
        Ok(summary) => summary,
        Err(error) => {
            record_sync_failure(&state, &connection.id, &error.to_string());
            return Err(sync_error(error));
        }
    };

    Ok(IntervalsSyncReport {
        inserted: summary.inserted,
        updated: summary.updated,
        unchanged: summary.unchanged,
        removed: summary.removed,
        unsupported: summary.unsupported,
        issues: summary
            .issues
            .into_iter()
            .map(|issue| format!("Event {}: {}", issue.external_event_id, issue.message))
            .collect(),
        oldest_date_local,
        newest_date_local,
    })
}

#[tauri::command]
pub async fn disconnect_intervals_icu(
    state: State<'_, AppState>,
) -> Result<IntervalsConnectionStatus, AppError> {
    let Some(connection) =
        provider_connections::get_active_by_provider(&state.db.lock().unwrap(), PROVIDER_ID)?
    else {
        return Ok(disconnected_status());
    };

    delete_api_key(state.credential_service.clone(), connection.id.clone()).await?;

    let disconnected_at_unix_ms = now_unix_ms() as i64;
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction()?;
    scheduled_workouts::remove_all_for_connection(
        &transaction,
        &connection.id,
        disconnected_at_unix_ms,
    )?;
    provider_connections::mark_disconnected(&transaction, &connection.id, disconnected_at_unix_ms)?;
    transaction.commit()?;
    Ok(disconnected_status())
}

fn credential_username(connection_id: &str) -> String {
    format!("{CREDENTIAL_USERNAME_PREFIX}{connection_id}")
}

async fn store_api_key(
    service: String,
    connection_id: String,
    api_key: String,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        keyring::Entry::new(&service, &credential_username(&connection_id))
            .and_then(|entry| entry.set_password(&api_key))
            .map_err(credential_error)
    })
    .await
    .map_err(|error| AppError::new("credential_store", error.to_string()))?
}

async fn load_api_key(service: String, connection_id: String) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        keyring::Entry::new(&service, &credential_username(&connection_id))
            .and_then(|entry| entry.get_password())
            .map_err(credential_error)
    })
    .await
    .map_err(|error| AppError::new("credential_store", error.to_string()))?
}

async fn delete_api_key(service: String, connection_id: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let entry = keyring::Entry::new(&service, &credential_username(&connection_id))
            .map_err(credential_error)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(credential_error(error)),
        }
    })
    .await
    .map_err(|error| AppError::new("credential_store", error.to_string()))?
}

fn credential_error(error: keyring::Error) -> AppError {
    let message = match error {
        keyring::Error::NoEntry => {
            "The Intervals.icu API key is missing from the system credential store".into()
        }
        other => format!("Could not access the system credential store: {other}"),
    };
    AppError::new("credential_store", message)
}

fn record_sync_failure(state: &State<'_, AppState>, connection_id: &str, message: &str) {
    let _ = provider_connections::record_sync_failure(
        &state.db.lock().unwrap(),
        connection_id,
        message,
        now_unix_ms() as i64,
    );
}

fn api_error(error: IntervalsApiError) -> AppError {
    let code = match &error {
        IntervalsApiError::MissingApiKey | IntervalsApiError::Unauthorized => "intervals_auth",
        IntervalsApiError::Request(_) => "intervals_network",
        IntervalsApiError::HttpStatus { .. } => "intervals_http",
        IntervalsApiError::InvalidResponse(_) | IntervalsApiError::InvalidAthlete { .. } => {
            "intervals_response"
        }
        IntervalsApiError::InvalidDateRange { .. } => "intervals_date_range",
        IntervalsApiError::Client(_) => "intervals_client",
    };
    AppError::new(code, error.to_string())
}

fn sync_error(error: IntervalsSyncError) -> AppError {
    match error {
        IntervalsSyncError::Database(error) => AppError::from(error),
        other => AppError::new("intervals_sync", other.to_string()),
    }
}

fn sync_window(today_date_local: &str) -> Result<(String, String), AppError> {
    let invalid_date = || {
        AppError::new(
            "intervals_date_range",
            format!("Invalid athlete-local date {today_date_local:?}"),
        )
    };
    Ok((
        local_date::shift(today_date_local, -SYNC_LOOKBACK_DAYS).ok_or_else(invalid_date)?,
        local_date::shift(today_date_local, SYNC_LOOKAHEAD_DAYS).ok_or_else(invalid_date)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_window_crosses_month_and_leap_day_boundaries() {
        assert_eq!(
            sync_window("2028-02-28").unwrap(),
            ("2028-02-21".into(), "2028-04-10".into())
        );
        assert_eq!(
            sync_window("2026-01-03").unwrap(),
            ("2025-12-27".into(), "2026-02-14".into())
        );
    }

    #[test]
    fn sync_window_rejects_nonexistent_dates() {
        assert!(sync_window("2026-02-29").is_err());
        assert!(sync_window("2026-13-01").is_err());
        assert!(sync_window("20260917").is_err());
    }
}
