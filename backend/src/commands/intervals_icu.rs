//! Intervals.icu connection lifecycle and bounded schedule refresh commands.

use tauri::State;

use super::providers::{require_active, PlanSyncReport, ProviderOperations};
use crate::app_error::AppError;
use crate::app_state::{now_unix_ms, AppState};
use crate::database::{provider_connections, scheduled_workouts};
use crate::intervals_icu::{IntervalsApiError, IntervalsIcuClient, PROVIDER_ID};
use crate::intervals_icu_sync::{self, IntervalsSyncError};
use crate::local_date;

const SYNC_LOOKBACK_DAYS: i64 = 7;
const SYNC_LOOKAHEAD_DAYS: i64 = 42;
const CREDENTIAL_USERNAME_PREFIX: &str = "intervals_icu:";

#[tauri::command]
pub async fn connect_intervals_icu(
    state: State<'_, AppState>,
    operations: State<'_, ProviderOperations>,
    api_key: String,
) -> Result<String, AppError> {
    let _operation = operations.acquire(PROVIDER_ID)?;
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
                time_zone: Some(athlete.timezone.trim()),
                connected_at_unix_ms: now,
            },
        )?
    };
    Ok(connected_id)
}

pub async fn refresh(
    state: &AppState,
    connection: &provider_connections::ProviderConnectionRow,
) -> Result<PlanSyncReport, AppError> {
    let today_date_local =
        local_date::today_at(chrono::Utc::now(), connection.time_zone.as_deref());
    let (oldest_date_local, newest_date_local) = sync_window(&today_date_local)?;
    let api_key = match load_api_key(state.credential_service.clone(), connection.id.clone()).await
    {
        Ok(Some(api_key)) => api_key,
        Ok(None) => {
            let error = AppError::new(
                "provider_authentication",
                "Sign in to Intervals.icu in Settings → Connections.",
            );
            record_sync_failure(state, &connection.id, &error.message);
            return Err(error);
        }
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
            record_sync_failure(state, &connection.id, &error.to_string());
            if matches!(error, IntervalsApiError::Unauthorized) {
                delete_api_key(state.credential_service.clone(), connection.id.clone()).await?;
            }
            return Err(api_error(error));
        }
    };
    let synced_at_unix_ms = now_unix_ms() as i64;
    let persisted = intervals_icu_sync::persist_calendar_window(
        &mut state.db.lock().unwrap(),
        &connection.id,
        &oldest_date_local,
        &newest_date_local,
        &events,
        synced_at_unix_ms,
    );
    let summary = match persisted {
        Ok(summary) => summary,
        Err(error) => {
            record_sync_failure(&state, &connection.id, &error.to_string());
            return Err(sync_error(error));
        }
    };

    Ok(PlanSyncReport {
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

pub async fn disconnect(state: &AppState, connection_id: &str) -> Result<(), AppError> {
    let connection = require_active(&state.db.lock().unwrap(), connection_id, None)?;
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
    Ok(())
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

pub async fn has_credentials(state: &AppState, connection_id: &str) -> Result<bool, AppError> {
    Ok(
        load_api_key(state.credential_service.clone(), connection_id.to_owned())
            .await?
            .is_some(),
    )
}

async fn load_api_key(service: String, connection_id: String) -> Result<Option<String>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let entry = keyring::Entry::new(&service, &credential_username(&connection_id))
            .map_err(credential_error)?;
        match entry.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(credential_error(error)),
        }
    })
    .await
    .map_err(|_| {
        AppError::new(
            "credential_store",
            "Could not read Intervals.icu credentials.",
        )
    })?
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

fn record_sync_failure(state: &AppState, connection_id: &str, message: &str) {
    let _ = provider_connections::record_sync_failure(
        &state.db.lock().unwrap(),
        connection_id,
        message,
        now_unix_ms() as i64,
    );
}

fn api_error(error: IntervalsApiError) -> AppError {
    let code = match &error {
        IntervalsApiError::MissingApiKey | IntervalsApiError::Unauthorized => {
            "provider_authentication"
        }
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
