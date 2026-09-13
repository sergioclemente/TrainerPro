//! Shared pipeline for workout *sources* (the plugin layer).
//!
//! A source is any provider of ridable workouts: the local library, the
//! WorkoutPlanner server, whatsonzwift.com, future ones. The contract every
//! source reduces to: produce ZWO text, then hand it here — import (dedup'd),
//! tag provenance, load into the player. Sources never write their own
//! import/DB/player code.

use tauri::{AppHandle, Emitter, State};

use crate::app_error::AppError;
use crate::app_state::AppState;
use crate::commands::{player, workout};
use crate::database::workouts as workout_db;
use crate::player_runtime::PlayerState;

/// ZWO text → existing import pipeline (sha256 dedup) → provenance tag →
/// loaded player. `origin` is the source id ('planner', 'whatsonzwift'…);
/// `origin_ref` a source-local reference (wid, url slug…).
pub async fn ride_from_zwo(
    app: AppHandle,
    state: &State<'_, AppState>,
    zwo: &str,
    origin: &str,
    origin_ref: &str,
    origin_id: Option<i64>,
) -> Result<PlayerState, AppError> {
    let tmp_dir = state.data_dir.join("tmp");
    std::fs::create_dir_all(&tmp_dir)?;
    let safe: String = origin_ref
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .take(60)
        .collect();
    let tmp = tmp_dir.join(format!("{origin}-{safe}.zwo"));
    std::fs::write(&tmp, zwo)?;
    let import = workout::import_from_path(state, &tmp)?;
    let _ = std::fs::remove_file(&tmp);

    {
        let conn = state.db.lock().unwrap();
        workout_db::set_origin(
            &conn,
            &import.summary.id,
            origin,
            origin_id,
            origin_ref,
        )?;
    }
    for w in import.warnings.iter().take(3) {
        let _ = app.emit("toast", serde_json::json!({ "level": "warn", "message": w }));
    }
    player::do_load_workout(app.clone(), state, &import.summary.id).await
}
