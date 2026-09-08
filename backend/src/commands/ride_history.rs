//! Ride-history and FIT export IPC commands.

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::app_error::AppError;
use crate::app_state::AppState;

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
    let mut stmt = conn.prepare(
        "SELECT id, workout_name, started_at, timer_s, avg_power, np, tss, avg_hr,
                completed_pct, fit_path
         FROM rides ORDER BY started_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(RideRow {
                id: r.get(0)?,
                workout_name: r.get(1)?,
                started_at: r.get(2)?,
                timer_s: r.get(3)?,
                avg_power: r.get(4)?,
                np: r.get(5)?,
                tss: r.get(6)?,
                avg_hr: r.get(7)?,
                completed_pct: r.get(8)?,
                fit_path: r.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Deletes a ride from history: the SQLite row plus the app's own .fit and
/// journal files. Any copy you exported elsewhere (export folder, Save
/// As…) is left alone.
#[tauri::command]
pub async fn delete_ride(state: State<'_, AppState>, id: String) -> R<()> {
    let paths: Option<(String, String)> = {
        let conn = state.db.lock().unwrap();
        let p = conn
            .query_row(
                "SELECT fit_path, journal_path FROM rides WHERE id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        conn.execute("DELETE FROM rides WHERE id = ?1", [&id])?;
        p
    };
    if let Some((fit, journal)) = paths {
        let _ = std::fs::remove_file(fit);
        let _ = std::fs::remove_file(journal);
    }
    Ok(())
}

#[tauri::command]
pub async fn save_fit_as(state: State<'_, AppState>, id: String, dest_path: String) -> R<()> {
    let fit: String = {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT fit_path FROM rides WHERE id = ?1", [&id], |r| r.get(0))
            .map_err(|_| AppError::new("not_found", "ride not found"))?
    };
    std::fs::copy(fit, dest_path)?;
    Ok(())
}

#[tauri::command]
pub async fn reveal_fit(app: AppHandle, state: State<'_, AppState>, id: String) -> R<()> {
    let fit: String = {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT fit_path FROM rides WHERE id = ?1", [&id], |r| r.get(0))
            .map_err(|_| AppError::new("not_found", "ride not found"))?
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
