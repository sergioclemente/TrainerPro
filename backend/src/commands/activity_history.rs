//! Activity-history and FIT export IPC commands.

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::app_error::AppError;
use crate::app_state::AppState;
use crate::database::activities as activity_db;

type R<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize)]
pub struct ActivityRow {
    pub id: String,
    pub scheduled_workout_id: Option<String>,
    pub workout_name: String,
    pub started_at_unix_ms: i64,
    pub timer_s: u32,
    pub average_power_w: Option<u16>,
    pub normalized_power_w: Option<u16>,
    pub training_stress_score: Option<f64>,
    pub average_heart_rate_bpm: Option<u16>,
    pub completed_pct: f64,
    pub fit_path: String,
}

#[tauri::command]
pub async fn list_activities(state: State<'_, AppState>) -> R<Vec<ActivityRow>> {
    let conn = state.db.lock().unwrap();
    Ok(activity_db::list(&conn)?
        .into_iter()
        .map(ActivityRow::from)
        .collect())
}

impl From<activity_db::ActivityListRow> for ActivityRow {
    fn from(row: activity_db::ActivityListRow) -> Self {
        ActivityRow {
            id: row.id,
            scheduled_workout_id: row.scheduled_workout_id,
            workout_name: row.workout_name,
            started_at_unix_ms: row.started_at_unix_ms,
            timer_s: row.timer_s,
            average_power_w: row.average_power_w,
            normalized_power_w: row.normalized_power_w,
            training_stress_score: row.training_stress_score,
            average_heart_rate_bpm: row.average_heart_rate_bpm,
            completed_pct: row.completed_pct,
            fit_path: row.fit_path,
        }
    }
}

/// Deletes an activity from history: the SQLite row plus the app's own .fit and
/// journal files. Any copy you exported elsewhere (export folder, Save
/// As…) is left alone.
#[tauri::command]
pub async fn delete_activity(state: State<'_, AppState>, id: String) -> R<()> {
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
            .ok_or_else(|| AppError::new("not_found", "activity not found"))?
    };
    std::fs::copy(fit, dest_path)?;
    Ok(())
}

#[tauri::command]
pub async fn reveal_fit(app: AppHandle, state: State<'_, AppState>, id: String) -> R<()> {
    let fit: String = {
        let conn = state.db.lock().unwrap();
        activity_db::fit_path(&conn, &id)?
            .ok_or_else(|| AppError::new("not_found", "activity not found"))?
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
