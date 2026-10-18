//! Garmin authentication and capability adapters. The dispatcher owns transfer orchestration.

use super::providers::{require_active, PlanSyncReport};
use crate::app_error::AppError;
use crate::app_state::{now_unix_ms, AppState};
use crate::database::provider_connections;
use tp_integrations::garmin::{Account, GarminClient, GarminError, Tokens, PROVIDER_ID};

pub mod browser;
pub use browser::GarminSignInState;

const FIT_UPLOAD_FILENAME: &str = "TrainerPro.fit";

type R<T> = Result<T, AppError>;

pub async fn has_credentials(state: &AppState, connection_id: &str) -> R<bool> {
    Ok(
        load_tokens(state.credential_service.clone(), connection_id.to_owned())
            .await?
            .is_some(),
    )
}

async fn persist_connection(state: &AppState, tokens: Tokens, account: Account) -> R<()> {
    let connection_id = {
        let conn = state.db.lock().unwrap();
        if let Some(active) = provider_connections::get_active_by_provider(&conn, PROVIDER_ID)? {
            if active.external_account_id != account.id {
                return Err(AppError::new(
                    "garmin_account_conflict",
                    "Disconnect the current Garmin account before connecting another account.",
                ));
            }
        }
        provider_connections::get_by_provider_account(&conn, PROVIDER_ID, &account.id)?
            .map(|row| row.id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    };
    store_tokens(
        state.credential_service.clone(),
        connection_id.clone(),
        &tokens,
    )
    .await?;
    let connected = provider_connections::connect(
        &state.db.lock().unwrap(),
        &provider_connections::ConnectedProvider {
            id: &connection_id,
            provider: PROVIDER_ID,
            external_account_id: &account.id,
            display_name: account.display_name.as_deref(),
            time_zone: None,
            connected_at_unix_ms: now_unix_ms() as i64,
        },
    );
    if let Err(error) = connected {
        delete_tokens(state.credential_service.clone(), connection_id).await?;
        return Err(error.into());
    }
    Ok(())
}

// The capability dispatcher holds the provider guard before calling adapters.
pub async fn disconnect(
    state: &AppState,
    operations: &GarminSignInState,
    connection_id: &str,
) -> R<()> {
    operations.cancel().await;
    require_active(&state.db.lock().unwrap(), connection_id, None)?;
    delete_tokens(state.credential_service.clone(), connection_id.to_owned()).await?;
    let disconnected_at = now_unix_ms() as i64;
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction()?;
    crate::database::scheduled_workouts::remove_all_for_connection(
        &transaction,
        connection_id,
        disconnected_at,
    )?;
    provider_connections::mark_disconnected(&transaction, connection_id, disconnected_at)?;
    transaction.commit()?;
    Ok(())
}

pub async fn upload(state: &AppState, connection_id: &str, fit: Vec<u8>) -> R<String> {
    tp_integrations::garmin::validate_fit(&fit).map_err(api_error)?;
    let client = GarminClient::new().map_err(api_error)?;
    let tokens = fresh_tokens(state, connection_id, &client).await?;
    let remote_activity_id = match client.upload(&tokens, fit, FIT_UPLOAD_FILENAME).await {
        Ok(id) => id,
        Err(error) => {
            if error == GarminError::Authentication {
                delete_tokens(state.credential_service.clone(), connection_id.to_owned()).await?;
            }
            return Err(api_error(error));
        }
    };
    Ok(remote_activity_id)
}

/// Give the uploaded Activity the workout's name and description. Garmin
/// has already created it, so a failure here leaves a correctly recorded
/// upload with a default title; the caller reports, never retries the file.
pub async fn describe(
    state: &AppState,
    connection_id: &str,
    remote_activity_id: &str,
    name: &str,
    description: &str,
) -> R<()> {
    let client = GarminClient::new().map_err(api_error)?;
    let tokens = fresh_tokens(state, connection_id, &client).await?;
    client
        .describe_activity(&tokens, remote_activity_id, name, description)
        .await
        .map_err(api_error)
}

// Upload and schedule reads share refresh-token rotation and persistence under
// the existing provider guard. Do not start an API operation before saving it.
async fn fresh_tokens(state: &AppState, connection_id: &str, client: &GarminClient) -> R<Tokens> {
    let mut tokens = load_tokens(state.credential_service.clone(), connection_id.to_owned())
        .await?
        .ok_or_else(|| api_error(GarminError::Authentication))?;
    if tokens.needs_refresh() {
        tokens = match client.refresh(&tokens).await {
            Ok(tokens) => tokens,
            Err(error) => {
                if error == GarminError::Authentication {
                    delete_tokens(state.credential_service.clone(), connection_id.to_owned())
                        .await?;
                }
                return Err(api_error(error));
            }
        };
        // A rotated refresh token must be durable before any activity is sent.
        store_tokens(
            state.credential_service.clone(),
            connection_id.to_owned(),
            &tokens,
        )
        .await?;
    }
    Ok(tokens)
}

const SYNC_LOOKBACK_DAYS: i64 = 7;
const SYNC_LOOKAHEAD_DAYS: i64 = 42;

pub async fn refresh(
    state: &AppState,
    connection: &provider_connections::ProviderConnectionRow,
) -> R<PlanSyncReport> {
    let result = async {
        let today =
            crate::local_date::today_at(chrono::Utc::now(), connection.time_zone.as_deref());
        let oldest = crate::local_date::shift(&today, -SYNC_LOOKBACK_DAYS)
            .ok_or_else(|| AppError::new("garmin_date_range", "Invalid Garmin schedule date."))?;
        let newest = crate::local_date::shift(&today, SYNC_LOOKAHEAD_DAYS)
            .ok_or_else(|| AppError::new("garmin_date_range", "Invalid Garmin schedule date."))?;
        let client = GarminClient::new().map_err(api_error)?;
        let tokens = fresh_tokens(state, &connection.id, &client).await?;
        let events = crate::garmin_sync::fetch_calendar_window(&client, &tokens, &oldest, &newest)
            .await
            .map_err(api_error)?;
        crate::garmin_sync::persist_calendar_window(
            &mut state.db.lock().unwrap(),
            &connection.id,
            &oldest,
            &newest,
            &events,
            now_unix_ms() as i64,
        )
    }
    .await;
    if let Err(error) = &result {
        let _ = provider_connections::record_sync_failure(
            &state.db.lock().unwrap(),
            &connection.id,
            &error.message,
            now_unix_ms() as i64,
        );
        if error.code == "provider_authentication" {
            delete_tokens(state.credential_service.clone(), connection.id.clone()).await?;
        }
    }
    result
}

fn api_error(error: GarminError) -> AppError {
    let (code, message) = match error {
        GarminError::Network => ("garmin_network", "Could not reach Garmin. Check your connection and try again."),
        GarminError::Authentication => ("provider_authentication", "Garmin sign-in expired or was rejected. Sign in again in Settings → Connections."),
        GarminError::Challenge => ("garmin_challenge", "Garmin requires a browser challenge. Use manual FIT import and try connecting later."),
        GarminError::RateLimited => ("garmin_rate_limited", "Garmin is limiting requests. Wait before trying again."),
        GarminError::Protocol | GarminError::InvalidRequest | GarminError::ResponseTooLarge => ("garmin_protocol", "Garmin returned an unexpected response. Try again later or use manual FIT import."),
        GarminError::MfaRejected => ("garmin_mfa_rejected", "The Garmin verification code was rejected. Check the code and try again."),
        GarminError::MfaExpired => ("garmin_mfa_expired", "Garmin verification expired. Start sign-in again."),
        GarminError::Duplicate => ("activity_duplicate", "Garmin says this activity already exists. Check Garmin Connect before trying again."),
        GarminError::UploadRejected => ("garmin_upload_rejected", "Garmin rejected this FIT activity. Try importing the saved file on Garmin Connect."),
        GarminError::UploadConsentRequired => ("garmin_upload_consent_required", "Garmin requires an account step before uploads. Open Garmin Connect and review pending account setup or data-upload consent, then retry."),
        GarminError::UploadUncertain => ("activity_upload_uncertain", "Garmin may have received this activity, but its upload could not be confirmed. Check Garmin Connect before retrying."),
        GarminError::WorkoutUncertain => ("garmin_workout_uncertain", "Garmin could not confirm the workout operation. Check Garmin Connect before retrying."),
        GarminError::InvalidFit => ("garmin_invalid_fit", "The recorded file is not a FIT activity. Use another completed ride."),
    };
    AppError::new(code, message)
}

fn vault_error(_: keyring::Error) -> AppError {
    AppError::new(
        "credential_store",
        "Could not access Garmin credentials in the system credential manager.",
    )
}

async fn load_tokens(service: String, connection_id: String) -> R<Option<Tokens>> {
    tauri::async_runtime::spawn_blocking(move || {
        let entry = keyring::Entry::new(&service, &format!("garmin:{connection_id}"))
            .map_err(vault_error)?;
        match entry.get_password() {
            Ok(value) => Ok(serde_json::from_str(&value).ok()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(vault_error(error)),
        }
    })
    .await
    .map_err(|_| AppError::new("credential_store", "Could not read Garmin credentials."))?
}

async fn store_tokens(service: String, connection_id: String, tokens: &Tokens) -> R<()> {
    let value = serde_json::to_string(tokens)
        .map_err(|_| AppError::new("credential_store", "Could not encode Garmin credentials."))?;
    tauri::async_runtime::spawn_blocking(move || {
        keyring::Entry::new(&service, &format!("garmin:{connection_id}"))
            .and_then(|entry| entry.set_password(&value))
            .map_err(vault_error)
    })
    .await
    .map_err(|_| AppError::new("credential_store", "Could not store Garmin credentials."))?
}

async fn delete_tokens(service: String, connection_id: String) -> R<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let entry = keyring::Entry::new(&service, &format!("garmin:{connection_id}"))
            .map_err(vault_error)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(vault_error(error)),
        }
    })
    .await
    .map_err(|_| AppError::new("credential_store", "Could not remove Garmin credentials."))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_errors_preserve_the_ipc_contract() {
        for (error, code, message) in [
            (GarminError::Network, "garmin_network", "Could not reach Garmin. Check your connection and try again."),
            (GarminError::Authentication, "provider_authentication", "Garmin sign-in expired or was rejected. Sign in again in Settings → Connections."),
            (GarminError::Challenge, "garmin_challenge", "Garmin requires a browser challenge. Use manual FIT import and try connecting later."),
            (GarminError::RateLimited, "garmin_rate_limited", "Garmin is limiting requests. Wait before trying again."),
            (GarminError::Protocol, "garmin_protocol", "Garmin returned an unexpected response. Try again later or use manual FIT import."),
            (GarminError::MfaRejected, "garmin_mfa_rejected", "The Garmin verification code was rejected. Check the code and try again."),
            (GarminError::MfaExpired, "garmin_mfa_expired", "Garmin verification expired. Start sign-in again."),
            (GarminError::Duplicate, "activity_duplicate", "Garmin says this activity already exists. Check Garmin Connect before trying again."),
            (GarminError::UploadRejected, "garmin_upload_rejected", "Garmin rejected this FIT activity. Try importing the saved file on Garmin Connect."),
            (GarminError::UploadConsentRequired, "garmin_upload_consent_required", "Garmin requires an account step before uploads. Open Garmin Connect and review pending account setup or data-upload consent, then retry."),
            (GarminError::UploadUncertain, "activity_upload_uncertain", "Garmin may have received this activity, but its upload could not be confirmed. Check Garmin Connect before retrying."),
            (GarminError::InvalidFit, "garmin_invalid_fit", "The recorded file is not a FIT activity. Use another completed ride."),
        ] {
            let payload = serde_json::to_value(api_error(error)).unwrap();
            assert_eq!(payload, serde_json::json!({"code": code, "message": message}));
        }
    }
}
