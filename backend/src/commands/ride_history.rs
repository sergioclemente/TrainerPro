//! Ride-history and FIT export IPC commands.

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::app_error::AppError;
use crate::app_state::AppState;
use crate::database::activities as activity_db;

type R<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize)]
pub struct RideRow {
    pub id: String,
    pub workout_name: String,
    pub started_at: i64,
    pub timer_s: u32,
    pub avg_power: Option<u16>,
    pub np: Option<u16>,
    pub tss: Option<f64>,
    pub avg_hr: Option<u16>,
    pub completed_pct: f64,
    pub fit_path: String,
}

#[tauri::command]
pub async fn list_rides(state: State<'_, AppState>) -> R<Vec<RideRow>> {
    let conn = state.db.lock().unwrap();
    Ok(activity_db::list(&conn)?
        .into_iter()
        .map(RideRow::from)
        .collect())
}

impl From<activity_db::ActivityListRow> for RideRow {
    fn from(row: activity_db::ActivityListRow) -> Self {
        RideRow {
            id: row.id,
            workout_name: row.workout_name,
            started_at: row.started_at,
            timer_s: row.timer_s,
            avg_power: row.avg_power,
            np: row.np,
            tss: row.tss,
            avg_hr: row.avg_hr,
            completed_pct: row.completed_pct,
            fit_path: row.fit_path,
        }
    }
}

/// Deletes a ride from history: the SQLite row plus the app's own .fit and
/// journal files. Any copy you exported elsewhere (export folder, Save
/// As…) is left alone.
#[tauri::command]
pub async fn delete_ride(state: State<'_, AppState>, id: String) -> R<()> {
    let paths = {
        let conn = state.db.lock().unwrap();
        activity_db::delete(&conn, &id)?
    };
    if let Some(paths) = paths {
        let _ = std::fs::remove_file(paths.fit_path);
        let _ = std::fs::remove_file(paths.journal_path);
    }
    Ok(())
}

#[tauri::command]
pub async fn save_fit_as(state: State<'_, AppState>, id: String, dest_path: String) -> R<()> {
    let fit: String = {
        let conn = state.db.lock().unwrap();
        activity_db::fit_path(&conn, &id)?
            .ok_or_else(|| AppError::new("not_found", "ride not found"))?
    };
    std::fs::copy(fit, dest_path)?;
    Ok(())
}

#[tauri::command]
pub async fn reveal_fit(app: AppHandle, state: State<'_, AppState>, id: String) -> R<()> {
    let fit: String = {
        let conn = state.db.lock().unwrap();
        activity_db::fit_path(&conn, &id)?
            .ok_or_else(|| AppError::new("not_found", "ride not found"))?
    };
    app.opener()
        .reveal_item_in_dir(&fit)
        .map_err(|e| AppError::new("io", e.to_string()))
}

#[tauri::command]
pub async fn open_garmin_import(app: AppHandle) -> R<()> {
    app.opener()
        .open_url("https://connect.garmin.com/modern/import-data", None::<String>)
        .map_err(|e| AppError::new("io", e.to_string()))
}
