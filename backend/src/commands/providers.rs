//! Connection discovery and application operations for provider capabilities.
//! Provider adapters own authentication and wire protocols; this module owns
//! capability validation, transfer receipts, and coordination with local deletion.

use std::{collections::HashMap, future::Future, sync::Mutex as DbMutex};

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, MutexGuard};

use super::{garmin, intervals_icu};
use crate::app_error::AppError;
use crate::app_state::{now_unix_ms, AppState};
use crate::database::{activities, activity_uploads, provider_connections};

type R<T> = Result<T, AppError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapability {
    Planning,
    Activities,
}

#[derive(Serialize)]
pub struct ProviderDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub notice: Option<&'static str>,
    pub manual_import_url: Option<&'static str>,
    pub capabilities: &'static [ProviderCapability],
}

const PROVIDERS: &[ProviderDefinition] = &[
    ProviderDefinition {
        id: tp_integrations::garmin::PROVIDER_ID,
        name: "Garmin",
        description: "Upload completed rides from the ride summary or Activities.",
        notice: Some("Uses an unofficial Garmin connection. Uploaded rides may sync onward to services linked to your Garmin account."),
        manual_import_url: Some("https://connect.garmin.com/modern/import-data"),
        capabilities: &[ProviderCapability::Activities],
    },
    ProviderDefinition {
        id: tp_integrations::intervals_icu::PROVIDER_ID,
        name: "Intervals.icu",
        description: "Scheduled cycling workouts appear in Next Up and remain available offline.",
        notice: None,
        manual_import_url: None,
        capabilities: &[ProviderCapability::Planning],
    },
];

fn provider_definition(id: &str) -> R<&'static ProviderDefinition> {
    PROVIDERS
        .iter()
        .find(|definition| definition.id == id)
        .ok_or_else(|| AppError::new("provider_unknown", "This provider is not supported."))
}

/// One mutation per provider; independent providers do not block each other.
/// Every authentication command and capability mutation uses this same owner.
pub struct ProviderOperations(HashMap<&'static str, Mutex<()>>);

impl Default for ProviderOperations {
    fn default() -> Self {
        Self(
            PROVIDERS
                .iter()
                .map(|provider| (provider.id, Mutex::new(())))
                .collect(),
        )
    }
}

impl ProviderOperations {
    pub fn acquire(&self, provider_id: &str) -> R<MutexGuard<'_, ()>> {
        let definition = provider_definition(provider_id)?;
        self.0[definition.id].try_lock().map_err(|_| {
            AppError::new(
                "provider_busy",
                format!(
                    "A {} operation is already in progress. Wait for it to finish.",
                    definition.name
                ),
            )
        })
    }
}

/// Uploads and local deletion cannot overlap. When both guards are needed,
/// acquire this guard before ProviderOperations; never acquire in reverse order.
#[derive(Default)]
pub struct ActivityTransfers(Mutex<()>);

impl ActivityTransfers {
    pub fn acquire(&self) -> R<MutexGuard<'_, ()>> {
        self.0.try_lock().map_err(|_| {
            AppError::new(
                "activity_transfer_busy",
                "An Activity transfer is in progress. Wait for it to finish.",
            )
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderConnectionState {
    Disconnected,
    Connected,
    NeedsSignIn,
    Unavailable,
}

#[derive(Serialize)]
pub struct PlanSourceStatus {
    pub time_zone: Option<String>,
    pub last_sync_succeeded_at_unix_ms: Option<i64>,
    pub last_sync_error: Option<String>,
}

#[derive(Serialize)]
pub struct ActivityDestinationStatus {
    pub uploads: Vec<activity_uploads::ActivityUpload>,
}

#[derive(Serialize)]
pub struct ProviderConnection {
    pub provider: &'static ProviderDefinition,
    pub connection_id: Option<String>,
    pub state: ProviderConnectionState,
    pub external_account_id: Option<String>,
    pub display_name: Option<String>,
    pub error: Option<AppError>,
    pub plan_source: Option<PlanSourceStatus>,
    pub activity_destination: Option<ActivityDestinationStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanSyncReport {
    pub inserted: u32,
    pub updated: u32,
    pub unchanged: u32,
    pub removed: u32,
    pub unsupported: u32,
    pub issues: Vec<String>,
    pub oldest_date_local: String,
    pub newest_date_local: String,
}

#[tauri::command]
pub async fn list_provider_connections(state: State<'_, AppState>) -> R<Vec<ProviderConnection>> {
    let mut connections = Vec::new();
    for definition in PROVIDERS {
        let row =
            provider_connections::get_active_by_provider(&state.db.lock().unwrap(), definition.id)?;
        let mut view = ProviderConnection {
            provider: definition,
            connection_id: row.as_ref().map(|row| row.id.clone()),
            state: ProviderConnectionState::Disconnected,
            external_account_id: row.as_ref().map(|row| row.external_account_id.clone()),
            display_name: row.as_ref().and_then(|row| row.display_name.clone()),
            error: None,
            plan_source: definition
                .capabilities
                .contains(&ProviderCapability::Planning)
                .then(|| PlanSourceStatus {
                    time_zone: row.as_ref().and_then(|row| row.time_zone.clone()),
                    last_sync_succeeded_at_unix_ms: row
                        .as_ref()
                        .and_then(|row| row.last_sync_succeeded_at_unix_ms),
                    last_sync_error: row.as_ref().and_then(|row| row.last_sync_error.clone()),
                }),
            activity_destination: definition
                .capabilities
                .contains(&ProviderCapability::Activities)
                .then(|| ActivityDestinationStatus {
                    uploads: Vec::new(),
                }),
        };
        if let Some(row) = row {
            let credentials = match definition.id {
                tp_integrations::garmin::PROVIDER_ID => {
                    garmin::has_credentials(&state, &row.id).await
                }
                tp_integrations::intervals_icu::PROVIDER_ID => {
                    intervals_icu::has_credentials(&state, &row.id).await
                }
                _ => unreachable!("catalog must have an authentication adapter"),
            };
            match credentials {
                Ok(true) => view.state = ProviderConnectionState::Connected,
                Ok(false) => view.state = ProviderConnectionState::NeedsSignIn,
                Err(error) => {
                    view.state = ProviderConnectionState::Unavailable;
                    view.error = Some(error);
                }
            }
            if let Some(destination) = &mut view.activity_destination {
                destination.uploads = activity_uploads::list(&state.db.lock().unwrap(), &row.id)?;
            }
        }
        connections.push(view);
    }
    Ok(connections)
}

pub fn require_active(
    db: &Connection,
    connection_id: &str,
    capability: Option<ProviderCapability>,
) -> R<provider_connections::ProviderConnectionRow> {
    let connection = provider_connections::get(db, connection_id)?
        .filter(|row| row.disconnected_at_unix_ms.is_none())
        .ok_or_else(|| {
            AppError::new(
                "provider_not_connected",
                "This account is no longer connected. Reload Connections in Settings.",
            )
        })?;
    let definition = provider_definition(&connection.provider)?;
    if capability.is_some_and(|capability| !definition.capabilities.contains(&capability)) {
        return Err(AppError::new(
            "provider_capability_unsupported",
            "This connection does not support that operation.",
        ));
    }
    Ok(connection)
}

#[tauri::command]
pub async fn refresh_provider_plans(
    state: State<'_, AppState>,
    operations: State<'_, ProviderOperations>,
    connection_id: String,
) -> R<PlanSyncReport> {
    let connection = require_active(
        &state.db.lock().unwrap(),
        &connection_id,
        Some(ProviderCapability::Planning),
    )?;
    let _operation = operations.acquire(&connection.provider)?;
    let connection = require_active(
        &state.db.lock().unwrap(),
        &connection_id,
        Some(ProviderCapability::Planning),
    )?;
    match connection.provider.as_str() {
        tp_integrations::intervals_icu::PROVIDER_ID => {
            intervals_icu::refresh(&state, &connection).await
        }
        _ => unreachable!("catalog must have a plan-source adapter"),
    }
}

#[tauri::command]
pub async fn disconnect_provider(
    state: State<'_, AppState>,
    operations: State<'_, ProviderOperations>,
    garmin: State<'_, garmin::GarminSignInState>,
    connection_id: String,
) -> R<()> {
    let connection = require_active(&state.db.lock().unwrap(), &connection_id, None)?;
    let _operation = operations.acquire(&connection.provider)?;
    require_active(&state.db.lock().unwrap(), &connection_id, None)?;
    match connection.provider.as_str() {
        tp_integrations::garmin::PROVIDER_ID => {
            garmin::disconnect(&state, &garmin, &connection_id).await
        }
        tp_integrations::intervals_icu::PROVIDER_ID => {
            intervals_icu::disconnect(&state, &connection_id).await
        }
        _ => unreachable!("catalog must have a disconnect adapter"),
    }
}

#[tauri::command]
pub async fn upload_activity(
    state: State<'_, AppState>,
    operations: State<'_, ProviderOperations>,
    transfers: State<'_, ActivityTransfers>,
    connection_id: String,
    activity_id: String,
) -> R<activity_uploads::ActivityUpload> {
    let _transfer = transfers.acquire()?;
    let connection = require_active(
        &state.db.lock().unwrap(),
        &connection_id,
        Some(ProviderCapability::Activities),
    )?;
    let _operation = operations.acquire(&connection.provider)?;
    require_active(
        &state.db.lock().unwrap(),
        &connection_id,
        Some(ProviderCapability::Activities),
    )?;
    match connection.provider.as_str() {
        tp_integrations::garmin::PROVIDER_ID => {
            transfer_activity(&state.db, &connection_id, &activity_id, |fit| {
                garmin::upload(&state, &connection_id, fit)
            })
            .await
        }
        _ => unreachable!("catalog must have an activity-destination adapter"),
    }
}

// The dispatcher holds both guards across this transaction with the provider.
// The adapter consumes the artifact bytes and returns only a confirmed remote ID.
async fn transfer_activity<F, Fut>(
    db: &DbMutex<Connection>,
    connection_id: &str,
    activity_id: &str,
    send: F,
) -> R<activity_uploads::ActivityUpload>
where
    F: FnOnce(Vec<u8>) -> Fut,
    Fut: Future<Output = R<String>>,
{
    let fit_path = {
        let db = db.lock().unwrap();
        require_active(&db, connection_id, Some(ProviderCapability::Activities))?;
        if let Some(receipt) = activity_uploads::get(&db, connection_id, activity_id)? {
            return Ok(receipt);
        }
        activities::fit_path(&db, activity_id)?
            .ok_or_else(|| AppError::new("not_found", "Activity not found."))?
    };
    let fit = tauri::async_runtime::spawn_blocking(move || std::fs::read(fit_path))
        .await
        .map_err(|_| AppError::new("io", "Could not read the recorded FIT file."))?
        .map_err(|_| {
            AppError::new(
                "fit_missing",
                "The recorded FIT file is missing or unreadable.",
            )
        })?;
    let remote_activity_id = send(fit).await?;
    if remote_activity_id.trim().is_empty() {
        return Err(AppError::new("activity_upload_uncertain", "The provider did not confirm an Activity ID. Check its activity history before retrying."));
    }
    let receipt = activity_uploads::ActivityUpload {
        connection_id: connection_id.to_owned(),
        activity_id: activity_id.to_owned(),
        remote_activity_id,
        uploaded_at_unix_ms: now_unix_ms() as i64,
    };
    activity_uploads::record(&db.lock().unwrap(), &receipt)
        .map_err(|_| AppError::new("activity_receipt_failed", "The activity was uploaded, but TrainerPro could not save its upload record. Check the destination before retrying."))?;
    Ok(receipt)
}

#[tauri::command]
pub async fn open_activity_import(app: AppHandle, provider_id: String) -> R<()> {
    let url = provider_definition(&provider_id)?
        .manual_import_url
        .ok_or_else(|| {
            AppError::new(
                "provider_capability_unsupported",
                "This provider has no manual import page.",
            )
        })?;
    app.opener()
        .open_url(url, None::<String>)
        .map_err(|error| AppError::new("io", error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn connection(db: &Connection, id: &str, provider: &str, account: &str) {
        provider_connections::connect(
            db,
            &provider_connections::ConnectedProvider {
                id,
                provider,
                external_account_id: account,
                display_name: None,
                time_zone: Some("Europe/Zurich"),
                connected_at_unix_ms: 1,
            },
        )
        .unwrap();
    }

    #[test]
    fn capabilities_and_account_lifecycle_are_checked_before_dispatch() {
        let db = crate::database::test_connection();
        connection(&db, "plans", "intervals_icu", "i123");
        connection(&db, "uploads", "garmin", "123");
        assert!(require_active(&db, "plans", Some(ProviderCapability::Planning)).is_ok());
        assert!(require_active(&db, "uploads", Some(ProviderCapability::Activities)).is_ok());
        assert_eq!(
            require_active(&db, "plans", Some(ProviderCapability::Activities))
                .unwrap_err()
                .code,
            "provider_capability_unsupported"
        );
        assert_eq!(
            require_active(&db, "uploads", Some(ProviderCapability::Planning))
                .unwrap_err()
                .code,
            "provider_capability_unsupported"
        );
        provider_connections::mark_disconnected(&db, "uploads", 2).unwrap();
        connection(&db, "other-account", "garmin", "456");
        for id in ["uploads", "missing"] {
            assert_eq!(
                require_active(&db, id, None).unwrap_err().code,
                "provider_not_connected"
            );
        }
        assert_eq!(
            provider_definition("unknown").err().unwrap().code,
            "provider_unknown"
        );
    }

    #[test]
    fn mutations_serialize_per_provider_and_transfers_exclude_deletion() {
        let operations = ProviderOperations::default();
        let garmin = operations.acquire("garmin").unwrap();
        assert_eq!(
            operations.acquire("garmin").unwrap_err().code,
            "provider_busy"
        );
        assert!(operations.acquire("intervals_icu").is_ok());
        drop(garmin);
        assert!(operations.acquire("garmin").is_ok());
        let transfers = ActivityTransfers::default();
        let upload = transfers.acquire().unwrap();
        assert_eq!(
            transfers.acquire().unwrap_err().code,
            "activity_transfer_busy"
        );
        drop(upload);
        assert!(transfers.acquire().is_ok());
    }

    #[tokio::test]
    async fn transfer_records_only_confirmation_and_reuses_account_scoped_receipts() {
        let db = DbMutex::new(crate::database::test_connection());
        let fit = b"test recording bytes preserved exactly";
        let path =
            std::env::temp_dir().join(format!("trainerpro-transfer-{}.fit", uuid::Uuid::new_v4()));
        std::fs::write(&path, fit).unwrap();
        {
            let conn = db.lock().unwrap();
            connection(&conn, "uploads", "garmin", "123");
            conn.execute("INSERT INTO activities(id, workout_name, started_at_unix_ms, elapsed_s, timer_s, ftp_used_w, final_intensity_multiplier, fit_path, journal_path, completed_pct) VALUES ('ride', 'Test', 1, 30, 30, 200, 1, ?1, '/missing.jsonl', 100)", [path.to_str().unwrap()]).unwrap();
        }
        for code in ["activity_duplicate", "activity_upload_uncertain"] {
            let calls = Cell::new(0);
            let result = transfer_activity(&db, "uploads", "ride", |bytes| {
                assert_eq!(bytes, fit);
                calls.set(calls.get() + 1);
                std::future::ready(Err(AppError::new(code, "Unconfirmed")))
            })
            .await;
            assert_eq!(result.unwrap_err().code, code);
            assert_eq!(calls.get(), 1);
            assert!(activity_uploads::list(&db.lock().unwrap(), "uploads")
                .unwrap()
                .is_empty());
        }
        let result =
            transfer_activity(&db, "uploads", "ride", |_| async { Ok(String::new()) }).await;
        assert_eq!(result.unwrap_err().code, "activity_upload_uncertain");
        assert!(activity_uploads::list(&db.lock().unwrap(), "uploads")
            .unwrap()
            .is_empty());
        db.lock().unwrap().execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON activity_uploads BEGIN SELECT RAISE(FAIL, 'disk error'); END;").unwrap();
        let result =
            transfer_activity(&db, "uploads", "ride", |_| async { Ok("456".into()) }).await;
        assert_eq!(result.unwrap_err().code, "activity_receipt_failed");
        assert!(activity_uploads::list(&db.lock().unwrap(), "uploads")
            .unwrap()
            .is_empty());
        db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_receipt;")
            .unwrap();
        let receipt = transfer_activity(&db, "uploads", "ride", |bytes| async move {
            assert_eq!(bytes, fit);
            Ok("456".into())
        })
        .await
        .unwrap();
        assert_eq!(receipt.connection_id, "uploads");
        assert_eq!(receipt.remote_activity_id, "456");
        // A receipt needs neither the original artifact nor another request.
        std::fs::remove_file(&path).unwrap();
        let repeated = transfer_activity(&db, "uploads", "ride", |_| async {
            panic!("confirmed activity must not be sent again");
        })
        .await
        .unwrap();
        assert_eq!(receipt, repeated);
        {
            let conn = db.lock().unwrap();
            provider_connections::mark_disconnected(&conn, "uploads", 2).unwrap();
            connection(&conn, "other", "garmin", "789");
        }
        let result = transfer_activity(&db, "other", "ride", |_| async {
            panic!("missing file must fail before sending")
        })
        .await;
        assert_eq!(result.unwrap_err().code, "fit_missing");
        let result = transfer_activity(&db, "uploads", "ride", |_| async {
            panic!("stale account must fail before sending")
        })
        .await;
        assert_eq!(result.unwrap_err().code, "provider_not_connected");
    }
}
