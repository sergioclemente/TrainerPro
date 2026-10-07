//! Garmin authentication and capability adapters. The dispatcher owns transfer orchestration.

use serde::Serialize;
use tauri::State;
use tokio::sync::Mutex;

use super::providers::{require_active, ProviderOperations};
use crate::app_error::AppError;
use crate::app_state::{now_unix_ms, AppState};
use crate::database::provider_connections;
use tp_integrations::garmin::{GarminClient, GarminError, PendingMfa, SignIn, Tokens, PROVIDER_ID};

const FIT_UPLOAD_FILENAME: &str = "TrainerPro.fit";

type R<T> = Result<T, AppError>;

/// Transient Garmin MFA state. ProviderOperations serializes account mutations;
/// SQLite and the credential vault own durable state.
#[derive(Default)]
pub struct GarminSignInState(Mutex<Option<PendingMfa>>);

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GarminSignInResult {
    Connected,
    NeedsMfa,
}

pub async fn has_credentials(state: &AppState, connection_id: &str) -> R<bool> {
    Ok(
        load_tokens(state.credential_service.clone(), connection_id.to_owned())
            .await?
            .is_some(),
    )
}

#[tauri::command]
pub async fn connect_garmin(
    state: State<'_, AppState>,
    operations: State<'_, GarminSignInState>,
    providers: State<'_, ProviderOperations>,
    email: String,
    password: String,
) -> R<GarminSignInResult> {
    let _operation = providers.acquire(PROVIDER_ID)?;
    let mut pending = operations.0.lock().await;
    *pending = None;
    if email.trim().is_empty() || password.is_empty() {
        return Err(AppError::new(
            "garmin_credentials_required",
            "Enter your Garmin email and password.",
        ));
    }
    let result = GarminClient::new()
        .map_err(api_error)?
        .sign_in(email.trim(), &password)
        .await
        .map_err(api_error)?;
    // Neither the password nor email is retained by the pending MFA session.
    match result {
        SignIn::NeedsMfa(challenge) => {
            *pending = Some(challenge);
            Ok(GarminSignInResult::NeedsMfa)
        }
        SignIn::Connected(tokens) => {
            persist_connection(&state, tokens).await?;
            Ok(GarminSignInResult::Connected)
        }
    }
}

#[tauri::command]
pub async fn complete_garmin_mfa(
    state: State<'_, AppState>,
    operations: State<'_, GarminSignInState>,
    providers: State<'_, ProviderOperations>,
    code: String,
) -> R<()> {
    let _operation = providers.acquire(PROVIDER_ID)?;
    let mut pending = operations.0.lock().await;
    let challenge = pending
        .as_ref()
        .ok_or_else(|| api_error(GarminError::MfaExpired))?;
    let tokens = match challenge.complete(code.trim()).await {
        Ok(tokens) => tokens,
        Err(error) => {
            if error == GarminError::MfaExpired {
                *pending = None;
            }
            return Err(api_error(error));
        }
    };
    *pending = None;
    persist_connection(&state, tokens).await?;
    Ok(())
}

#[tauri::command]
pub async fn cancel_garmin_sign_in(
    operations: State<'_, GarminSignInState>,
    providers: State<'_, ProviderOperations>,
) -> R<()> {
    let _operation = providers.acquire(PROVIDER_ID)?;
    *operations.0.lock().await = None;
    Ok(())
}

async fn persist_connection(state: &AppState, tokens: Tokens) -> R<()> {
    let account = GarminClient::new()
        .map_err(api_error)?
        .account(&tokens)
        .await
        .map_err(api_error)?;
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
    let mut pending = operations.0.lock().await;
    *pending = None;
    require_active(&state.db.lock().unwrap(), connection_id, None)?;
    delete_tokens(state.credential_service.clone(), connection_id.to_owned()).await?;
    provider_connections::mark_disconnected(
        &state.db.lock().unwrap(),
        connection_id,
        now_unix_ms() as i64,
    )?;
    Ok(())
}

pub async fn upload(state: &AppState, connection_id: &str, fit: Vec<u8>) -> R<String> {
    tp_integrations::garmin::validate_fit(&fit).map_err(api_error)?;
    let mut tokens = load_tokens(state.credential_service.clone(), connection_id.to_owned())
        .await?
        .ok_or_else(|| api_error(GarminError::Authentication))?;
    let client = GarminClient::new().map_err(api_error)?;
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

fn api_error(error: GarminError) -> AppError {
    let (code, message) = match error {
        GarminError::Network => ("garmin_network", "Could not reach Garmin. Check your connection and try again."),
        GarminError::Authentication => ("provider_authentication", "Garmin sign-in expired or was rejected. Sign in again in Settings → Connections."),
        GarminError::Challenge => ("garmin_challenge", "Garmin requires a browser challenge. Use manual FIT import and try connecting later."),
        GarminError::RateLimited => ("garmin_rate_limited", "Garmin is limiting requests. Wait before trying again."),
        GarminError::Protocol => ("garmin_protocol", "Garmin returned an unexpected response. Try again later or use manual FIT import."),
        GarminError::MfaRejected => ("garmin_mfa_rejected", "The Garmin verification code was rejected. Check the code and try again."),
        GarminError::MfaExpired => ("garmin_mfa_expired", "Garmin verification expired. Start sign-in again."),
        GarminError::Duplicate => ("activity_duplicate", "Garmin says this activity already exists. Check Garmin Connect before trying again."),
        GarminError::UploadRejected => ("garmin_upload_rejected", "Garmin rejected this FIT activity. Try importing the saved file on Garmin Connect."),
        GarminError::UploadConsentRequired => ("garmin_upload_consent_required", "Garmin requires an account step before uploads. Open Garmin Connect and review pending account setup or data-upload consent, then retry."),
        GarminError::UploadUncertain => ("activity_upload_uncertain", "Garmin may have received this activity, but its upload could not be confirmed. Check Garmin Connect before retrying."),
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
