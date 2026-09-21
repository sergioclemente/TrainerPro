//! Shared pipeline for workout *sources* (the plugin layer).
//!
//! A source is any provider of ridable workouts: the local library, the
//! WorkoutPlanner server, whatsonzwift.com, or future connectors. Sources
//! normalize into TPW, then share provenance and player-loading behavior.

use tauri::{AppHandle, Emitter, State};
use tp_core::workout_definition::WorkoutDefinition;

use crate::app_error::AppError;
use crate::app_state::AppState;
use crate::commands::{player, workout};
use crate::database::workout_definitions as definition_db;
use crate::player_runtime::PlayerState;

/// ZWO boundary payload → TPW import → provenance tag → loaded player.
pub async fn ride_from_zwo(
    app: AppHandle,
    state: &State<'_, AppState>,
    zwo: &str,
    origin: &str,
    origin_ref: &str,
    origin_id: Option<i64>,
) -> Result<PlayerState, AppError> {
    let import = workout::import_content(state, zwo, "zwo", Vec::new())?;
    tag_and_load(app, state, import, origin, origin_ref, origin_id).await
}

/// Already-normalized TPW → provenance tag → loaded player.
pub async fn ride_from_definition(
    app: AppHandle,
    state: &State<'_, AppState>,
    definition: &WorkoutDefinition,
    origin: &str,
    origin_ref: &str,
    origin_id: Option<i64>,
) -> Result<PlayerState, AppError> {
    let import = workout::store_definition(state, definition, Vec::new())?;
    tag_and_load(app, state, import, origin, origin_ref, origin_id).await
}

async fn tag_and_load(
    app: AppHandle,
    state: &State<'_, AppState>,
    import: workout::ImportResult,
    origin: &str,
    origin_ref: &str,
    origin_id: Option<i64>,
) -> Result<PlayerState, AppError> {
    // Deduplication may reuse a definition from another local source. Preserve
    // that row's existing provenance instead of relabeling it as this source.
    if !import.already_existed {
        let conn = state.db.lock().unwrap();
        definition_db::set_origin(
            &conn,
            &import.summary.id,
            origin,
            origin_id,
            origin_ref,
            crate::app_state::now_unix_ms() as i64,
        )?;
    }
    for w in import.warnings.iter().take(3) {
        let _ = app.emit("toast", serde_json::json!({ "level": "warn", "message": w }));
    }
    player::load_workout_into_player(app.clone(), state, &import.summary.id, None).await
}
